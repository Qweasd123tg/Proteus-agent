// OpenCode notifications are not part of the Proteus tool result contract.
export async function sendCompressNotification() {}
export async function sendIgnoredMessage(client, _session, text) {
  if (!client.commandMessage) throw new Error("DCP command output outside command invocation");
  client.commandMessage(text);
}
