export class Logger {
  constructor(debug = false) { this.enabled = debug; }
  debug(message, details) { if (this.enabled) console.error(message, details ?? ""); }
  info(message, details) { this.debug(message, details); }
  warn(message, details) { console.error(message, details ?? ""); }
  error(message, details) { console.error(message, details ?? ""); }
  async saveContext() {}
}
