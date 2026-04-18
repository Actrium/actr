/**
 * Actr entry point
 *
 * Config is loaded dynamically from /actr-runtime-config.json.
 * Call initConfig() before using other exports.
 */

export {
    initConfig,
    actrConfig,
    buildActrConfig,
    buildRuntimeConfig,
    runtimeConfig,
    actrType,
    system,
    acl,
} from './actr-config';

export type { RuntimeConfigJson } from './actr-config';
