/**
 * Removes NAPI_RS_ENFORCE_VERSION_CHECK and the binding version check blocks
 * from napi-rs generated index.js. CI regenerates index.js and runs this so
 * the published package does not depend on that env var.
 *
 * Run: node scripts/strip-napi-version-check.js [path to index.js]
 * Default path: ./index.js
 */

const { readFileSync, writeFileSync } = require('node:fs');
const path = require('node:path');

const indexPath = path.resolve(process.cwd(), process.argv[2] || 'index.js');
let content = readFileSync(indexPath, 'utf8');

// Remove each 4-line block: bindingPackageVersion, if (..NAPI_RS_ENFORCE_VERSION_CHECK..), throw, }
// Indentation and version string may vary; match greedily per block.
// Match up to closing brace and newline only, so next line's indentation is preserved
const blockPattern = /\n\s*const bindingPackageVersion = require\([^)]+\)\.version\s*\n\s*if \(bindingPackageVersion !== '[^']+' && process\.env\.NAPI_RS_ENFORCE_VERSION_CHECK && process\.env\.NAPI_RS_ENFORCE_VERSION_CHECK !== '0'\) \{\s*\n\s*throw new Error\([^)]+\)\s*\n\s*\}\n/g;

const newContent = content.replace(blockPattern, '\n');
if (newContent === content) {
  console.warn('strip-napi-version-check: no version check blocks found');
  process.exit(0);
}

writeFileSync(indexPath, newContent, 'utf8');
console.log('strip-napi-version-check: removed NAPI_RS_ENFORCE_VERSION_CHECK blocks from', indexPath);
