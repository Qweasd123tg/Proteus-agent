// Only the current adapter format is accepted before upstream hydrates its maps.
const object = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const exact = (value, required, optional = []) => object(value) && required.every((key) => key in value) &&
  Object.keys(value).every((key) => required.includes(key) || optional.includes(key));
const number = (value) => Number.isFinite(value) && value >= 0;
const id = (value) => Number.isSafeInteger(value) && value > 0;
const array = (value, check) => Array.isArray(value) && value.every(check);
const record = (value, check) => object(value) && Object.values(value).every(check);
const string = (value) => typeof value === "string";
function block(value) {
  const numbers = ["compressedTokens", "summaryTokens", "durationMs", "createdAt"];
  const strings = ["topic", "startId", "endId", "anchorMessageId", "compressMessageId", "summary"];
  const ids = ["includedBlockIds", "consumedBlockIds", "parentBlockIds"];
  const messages = ["directMessageIds", "directToolIds", "effectiveMessageIds", "effectiveToolIds"];
  return exact(value, ["blockId", "runId", "active", "deactivatedByUser", "mode", ...numbers, ...strings, ...ids, ...messages],
    ["batchTopic", "compressCallId", "deactivatedAt", "deactivatedByBlockId"]) &&
    id(value.blockId) && id(value.runId) && typeof value.active === "boolean" && typeof value.deactivatedByUser === "boolean" &&
    ["range", "message"].includes(value.mode) && numbers.every((key) => number(value[key])) && strings.every((key) => string(value[key])) &&
    ids.every((key) => array(value[key], id)) && messages.every((key) => array(value[key], string)) &&
    ["batchTopic", "compressCallId"].every((key) => !(key in value) || string(value[key])) &&
    (!("deactivatedAt" in value) || number(value.deactivatedAt)) && (!("deactivatedByBlockId" in value) || id(value.deactivatedByBlockId));
}
export function validateEnvelope(envelope) {
  const state = envelope?.state;
  const prune = state?.prune;
  const messages = prune?.messages;
  const valid = exact(envelope, ["version", "state"]) && envelope.version === 1 &&
    exact(state, ["manualMode", "prune", "nudges", "stats", "lastUpdated"]) && state.manualMode === false && string(state.lastUpdated) &&
    exact(state.stats, ["pruneTokenCounter", "totalPruneTokens"]) && Object.values(state.stats).every(number) &&
    exact(state.nudges, ["contextLimitAnchors", "turnNudgeAnchors", "iterationNudgeAnchors"]) && Object.values(state.nudges).every((v) => array(v, string)) &&
    exact(prune, ["tools", "messages"]) && record(prune.tools, number) &&
    exact(messages, ["byMessageId", "blocksById", "activeBlockIds", "activeByAnchorMessageId", "nextBlockId", "nextRunId"]) &&
    id(messages.nextBlockId) && id(messages.nextRunId) && array(messages.activeBlockIds, id) && record(messages.activeByAnchorMessageId, id) &&
    record(messages.byMessageId, (entry) => exact(entry, ["tokenCount", "allBlockIds", "activeBlockIds"]) &&
      number(entry.tokenCount) && array(entry.allBlockIds, id) && array(entry.activeBlockIds, id)) && record(messages.blocksById, block) &&
    Object.entries(messages.blocksById).every(([key, value]) => String(value.blockId) === key);
  if (!valid) throw new Error("invalid DCP persisted state");
  return state;
}
