import { spawn } from "node:child_process";

/** Run an existing JSON-stdin script; callers explicitly map its payload/decision. */
export async function runCommand(command, args, input, ctx) {
  ctx.signal.throwIfAborted();
  return new Promise((resolve, reject) => {
    const grouped = process.platform !== "win32";
    const child = spawn(command, args, {
      cwd: ctx.cwd, detached: grouped, stdio: ["pipe", "pipe", "pipe"],
    });
    const stdout = [];
    const stderr = [];
    let size = 0;
    let failure;
    const stop = () => {
      if (!child.pid) return;
      try {
        if (grouped) process.kill(-child.pid, "SIGKILL");
        else child.kill("SIGKILL");
      } catch (error) {
        if (error.code !== "ESRCH") failure ??= error;
      }
    };
    ctx.signal.addEventListener("abort", stop, { once: true });
    // Abort may race with process creation before the listener is installed.
    if (ctx.signal.aborted) stop();
    const receive = (chunks) => (chunk) => {
      size += chunk.length;
      if (size > 1_048_576) {
        failure = new Error("command hook output exceeds 1 MiB");
        stop();
      } else chunks.push(chunk);
    };
    child.stdout.on("data", receive(stdout));
    child.stderr.on("data", receive(stderr));
    child.on("error", (error) => { failure = error; });
    child.stdin.on("error", (error) => { failure ??= error; stop(); });
    // Descendants may retain stdout after the script exits; they belong to this invocation.
    child.on("exit", stop);
    child.on("close", (code, signal) => {
      ctx.signal.removeEventListener("abort", stop);
      if (ctx.signal.aborted) return reject(ctx.signal.reason);
      if (failure) return reject(failure);
      if (signal) return reject(new Error(`command hook terminated by ${signal}`));
      resolve({ code, stdout: Buffer.concat(stdout).toString("utf8"), stderr: Buffer.concat(stderr).toString("utf8") });
    });
    child.stdin.end(`${JSON.stringify(input)}\n`);
  });
}
