use anyhow::{Result, bail};
use proteus_core::app_server::a2a::A2aServerConfig;

pub(super) fn parse(task: &[String]) -> Result<Option<A2aServerConfig>> {
    let [server, transport, rest @ ..] = task else {
        return Ok(None);
    };
    if server != "server" || transport != "a2a" {
        return Ok(None);
    }
    let mut config = A2aServerConfig::default();
    let mut args = rest.iter();
    let mut port_seen = false;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--ready-stdout" if !config.ready_stdout => config.ready_stdout = true,
            "--port" if !port_seen => {
                config.port = args
                    .next()
                    .ok_or_else(|| anyhow::anyhow!("--port requires a value"))?
                    .parse()?;
                port_seen = true;
            }
            _ => bail!("usage: proteus [options] server a2a [--port <port>] [--ready-stdout]"),
        }
    }
    Ok(Some(config))
}
