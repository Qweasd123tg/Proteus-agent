import { clientWorkspace } from "../ui/workspace/client.js";
export { mountClientWorkspace, revealClientView } from "../ui/workspace/client.js";
import { mountExtensions } from "./host.js";
import { createExtensionRegistry } from "./registry.js";
import { attachClientTabHost } from "./subagent-tabs.js";

import { createAgentServices } from "./agent-services.js";
export { publishSessionState } from "./session-state.js";

import { builtins } from "../ui/modules/catalog.js";
import { createClientModuleRegistry } from "../ui/modules/registry.js";
import { mountSettings } from "../ui/modules/settings-host.js";
import { mountComposerSlot } from "../ui/modules/host.js";
import { moduleServices, readRequestedModule } from "../ui/modules/services.js";
export {
  configureModules,
  publishModules,
  requestSettingsModule,
} from "../ui/modules/services.js";
const registry = createClientModuleRegistry(
  createExtensionRegistry({ reservedIds: builtins.map((r) => r.id) }),
);
const agent = createAgentServices();
const clientServices = Object.assign(moduleServices(registry), agent.services);
export function mountClientSettings(root) {
  return mountSettings(root, registry, clientServices, readRequestedModule());
}
export function mountClientSlot(root, slot) {
  return mountComposerSlot(root, slot, registry, clientServices);
}

// Адаптер этой витрины. Credentials остаются в transport-коде клиента;
// расширение получает только объявленный интерфейс чтения публичного API.
export function mountWebExtensions(
  root,
  readConfig,
  readQuota,
  readUsage,
  readWorkspace,
  readConfigBuilder,
  saveConfigBuilder,
) {
  const release = agent.bind({
    readConfig,
    readQuota,
    readUsage,
    readWorkspace,
    readConfigBuilder,
    saveConfigBuilder,
  });
  const stop = mountExtensions(root, clientServices, {
    registry,
    workspace: clientWorkspace(),
    clientTabs: attachClientTabHost,
  });
  return () => {
    stop();
    release();
  };
}
