import { mountReport } from './report.js';
import { icon } from '../dom.js';
export function mount(context) { icon(context.compact, 'analysis'); return mountReport(context); }
