import { mountAgentPage } from "./page.js";
import { slotSection } from "./choices.js";
import { slotText } from "./labels.js";

const slot = new URL(import.meta.url).searchParams.get("slot");

export function mount(context) {
  const [, description] = slotText[slot] ?? ["", ""];
  mountAgentPage(context, description, (body, snapshot, view) =>
    slotSection(body, snapshot, slot, view),
  );
}
