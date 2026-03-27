/**
 * Web Package Loader — Web equivalent of Rust Hyper's load_package_executor
 *
 * In Rust Hyper, load_package_executor:
 *   1. Takes bytes of a .actr package (signed ZIP)
 *   2. Verifies the signature
 *   3. Extracts the WASM binary
 *   4. Loads it with wasmtime
 *   5. Returns an executor
 *
 * In the Web platform, the equivalent is:
 *   1. Fetch .actr package from a URL (or accept raw bytes)
 *   2. Parse the ZIP (STORE method, no compression)
 *   3. Extract manifest.toml manifest, WASM binary, JS glue, and actor.sw.js
 *   4. Return structured package contents for inspection or SW loading
 *
 * The actual WASM loading in the Service Worker happens via actor.sw.js
 * which has its own ZIP parser and loads directly from package_url.
 * This module provides main-thread inspection of .actr packages.
 *
 * .actr ZIP structure (all entries STORE / no compression):
 *   manifest.toml             - package manifest (TOML)
 *   manifest.sig              - Ed25519 signature (64 bytes)
 *   bin/actor.wasm        - WASM binary
 *   resources/glue.js     - wasm-bindgen JS glue
 *   resources/actor.sw.js - Service Worker entry
 */

/**
 * Parsed manifest.toml manifest
 */
export interface ActrManifest {
  manufacturer: string;
  name: string;
  version: string;
  signature_algorithm: string;
  binary: {
    path: string;
    target: string;
    hash: string;
    size: number;
  };
  resources: Array<{
    path: string;
    hash: string;
  }>;
  metadata: {
    description: string;
    license: string;
  };
}

/**
 * Result of parsing a .actr package
 */
export interface LoadedActrPackage {
  /** Parsed manifest from manifest.toml */
  manifest: ActrManifest;
  /** Raw WASM binary bytes */
  wasmBytes: Uint8Array;
  /** JS glue source text (resources/glue.js) */
  jsGlue: string | null;
  /** actor.sw.js source text */
  actorSwJs: string | null;
  /** All files in the ZIP: filename → bytes */
  files: Map<string, Uint8Array>;
}

// ── ZIP parser (STORE-only) ──

/**
 * Parse a ZIP file with STORE compression (no deflate).
 * .actr packages always use CompressionMethod::Stored.
 */
function parseStoreZip(buffer: ArrayBuffer): Map<string, Uint8Array> {
  const view = new DataView(buffer);
  const bytes = new Uint8Array(buffer);
  const entries = new Map<string, Uint8Array>();
  let offset = 0;

  while (offset + 30 <= buffer.byteLength) {
    const sig = view.getUint32(offset, true);
    if (sig !== 0x04034b50) break; // not a Local File Header

    const compressedSize = view.getUint32(offset + 18, true);
    const filenameLen = view.getUint16(offset + 26, true);
    const extraLen = view.getUint16(offset + 28, true);

    const filenameBytes = bytes.subarray(offset + 30, offset + 30 + filenameLen);
    const filename = new TextDecoder().decode(filenameBytes);

    const dataStart = offset + 30 + filenameLen + extraLen;
    const dataEnd = dataStart + compressedSize;

    if (dataEnd > buffer.byteLength) break;

    entries.set(filename, bytes.slice(dataStart, dataEnd));
    offset = dataEnd;
  }

  return entries;
}

// ── Minimal TOML parser for manifest.toml ──

/**
 * Parse a minimal subset of TOML used in manifest.toml.
 * Handles top-level keys, [section] headers, [[array]] items.
 */
function parseActrToml(text: string): ActrManifest {
  const manifest: ActrManifest = {
    manufacturer: '',
    name: '',
    version: '',
    signature_algorithm: '',
    binary: { path: '', target: '', hash: '', size: 0 },
    resources: [],
    metadata: { description: '', license: '' },
  };

  let currentSection = '';
  let currentResource: { path: string; hash: string } | null = null;

  for (const rawLine of text.split('\n')) {
    const line = rawLine.trim();
    if (!line || line.startsWith('#')) continue;

    // [[resources]] array
    if (line === '[[resources]]') {
      if (currentResource) manifest.resources.push(currentResource);
      currentResource = { path: '', hash: '' };
      currentSection = 'resources';
      continue;
    }

    // [section] header
    const sectionMatch = line.match(/^\[(\w+)\]$/);
    if (sectionMatch) {
      if (currentResource) {
        manifest.resources.push(currentResource);
        currentResource = null;
      }
      currentSection = sectionMatch[1];
      continue;
    }

    // key = value
    const kvMatch = line.match(/^(\w+)\s*=\s*(.+)$/);
    if (!kvMatch) continue;

    const key = kvMatch[1];
    let val = kvMatch[2].trim();
    // Strip quotes
    if ((val.startsWith('"') && val.endsWith('"')) || (val.startsWith("'") && val.endsWith("'"))) {
      val = val.slice(1, -1);
    }

    if (currentSection === '' || currentSection === 'package') {
      if (key === 'manufacturer') manifest.manufacturer = val;
      else if (key === 'name') manifest.name = val;
      else if (key === 'version') manifest.version = val;
      else if (key === 'signature_algorithm') manifest.signature_algorithm = val;
    } else if (currentSection === 'binary') {
      if (key === 'path') manifest.binary.path = val;
      else if (key === 'target') manifest.binary.target = val;
      else if (key === 'hash') manifest.binary.hash = val;
      else if (key === 'size') manifest.binary.size = parseInt(val, 10);
    } else if (currentSection === 'resources' && currentResource) {
      if (key === 'path') currentResource.path = val;
      else if (key === 'hash') currentResource.hash = val;
    } else if (currentSection === 'metadata') {
      if (key === 'description') manifest.metadata.description = val;
      else if (key === 'license') manifest.metadata.license = val;
    }
  }

  if (currentResource) manifest.resources.push(currentResource);

  return manifest;
}

// ── Public API ──

/**
 * Fetch and parse a .actr package from a URL.
 *
 * This is the main-thread equivalent of Hyper's load_package_executor.
 * Use this for package inspection, manifest reading, or pre-validation
 * before the Service Worker loads the package.
 *
 * @param url - URL of the .actr package
 * @returns Parsed package contents
 */
export async function loadActrPackage(url: string): Promise<LoadedActrPackage> {
  const resp = await fetch(url);
  if (!resp.ok) {
    throw new Error(`Failed to fetch .actr package: ${resp.status} ${resp.statusText} (${url})`);
  }
  const buffer = await resp.arrayBuffer();
  return parseActrPackage(buffer);
}

/**
 * Parse raw .actr package bytes (ArrayBuffer).
 *
 * Extracts all ZIP entries, parses manifest.toml manifest,
 * and returns structured access to WASM binary, JS glue, etc.
 *
 * @param buffer - Raw .actr ZIP bytes
 * @returns Parsed package contents
 */
export function parseActrPackage(buffer: ArrayBuffer): LoadedActrPackage {
  const files = parseStoreZip(buffer);

  // Parse manifest
  const tomlBytes = files.get('manifest.toml');
  if (!tomlBytes) {
    throw new Error('Invalid .actr package: missing manifest.toml');
  }
  const manifest = parseActrToml(new TextDecoder().decode(tomlBytes));

  // Find WASM binary
  let wasmBytes: Uint8Array | null = null;
  for (const [name, data] of files) {
    if (name.startsWith('bin/') && name.endsWith('.wasm')) {
      wasmBytes = data;
      break;
    }
  }
  if (!wasmBytes) {
    throw new Error('Invalid .actr package: no WASM binary found');
  }

  // Find JS glue (resources/*.js, not actor.sw.js)
  let jsGlue: string | null = null;
  for (const [name, data] of files) {
    if (name.startsWith('resources/') && name.endsWith('.js') && !name.endsWith('actor.sw.js')) {
      jsGlue = new TextDecoder().decode(data);
      break;
    }
  }

  // Find actor.sw.js
  let actorSwJs: string | null = null;
  const swBytes = files.get('resources/actor.sw.js');
  if (swBytes) {
    actorSwJs = new TextDecoder().decode(swBytes);
  }

  return { manifest, wasmBytes, jsGlue, actorSwJs, files };
}
