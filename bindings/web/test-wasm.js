#!/usr/bin/env node

/**
 * Simple WASM Test Script
 * Tests WASM loading and basic functionality without browser
 */

import fs from 'fs';
import path from 'path';
import { fileURLToPath } from 'url';

const __filename = fileURLToPath(import.meta.url);
const __dirname = path.dirname(__filename);

console.log('🧪 Actor-RTC Web - WASM Testing\n');
console.log('=' .repeat(50));

const testResults = [];

function test(name, fn) {
    try {
        const result = fn();
        testResults.push({ name, passed: true, result });
        console.log(`✅ ${name}`);
        if (result) console.log(`   └─ ${result}`);
        return true;
    } catch (error) {
        testResults.push({ name, passed: false, error: error.message });
        console.log(`❌ ${name}`);
        console.log(`   └─ Error: ${error.message}`);
        return false;
    }
}

async function asyncTest(name, fn) {
    try {
        const result = await fn();
        testResults.push({ name, passed: true, result });
        console.log(`✅ ${name}`);
        if (result) console.log(`   └─ ${result}`);
        return true;
    } catch (error) {
        testResults.push({ name, passed: false, error: error.message });
        console.log(`❌ ${name}`);
        console.log(`   └─ Error: ${error.message}`);
        return false;
    }
}

// Test 1: Check WASM file exists
console.log('\n📦 File System Tests:');
const wasmPath = path.join(__dirname, 'packages/web-runtime/src/actr_platform_web_bg.wasm');
test('WASM file exists', () => {
    const exists = fs.existsSync(wasmPath);
    if (!exists) throw new Error('WASM file not found');
    return `Path: ${wasmPath}`;
});

// Test 2: Check WASM file size
test('WASM file size', () => {
    const stats = fs.statSync(wasmPath);
    const sizeKB = (stats.size / 1024).toFixed(1);
    if (stats.size === 0) throw new Error('WASM file is empty');
    return `${sizeKB} KB`;
});

// Test 3: Check JavaScript bindings
const jsPath = path.join(__dirname, 'packages/web-runtime/src/actr_platform_web.js');
test('JavaScript bindings exist', () => {
    const exists = fs.existsSync(jsPath);
    if (!exists) throw new Error('JS bindings not found');
    const content = fs.readFileSync(jsPath, 'utf8');
    return `${content.length} bytes`;
});

// Test 4: Check TypeScript definitions
const dtsPath = path.join(__dirname, 'packages/web-runtime/src/actr_platform_web.d.ts');
test('TypeScript definitions exist', () => {
    const exists = fs.existsSync(dtsPath);
    if (!exists) throw new Error('TS definitions not found');
    return `Path: ${dtsPath}`;
});

// Test 5: Validate WASM binary
console.log('\n🔍 WASM Validation Tests:');
await asyncTest('WASM binary is valid', async () => {
    const buffer = fs.readFileSync(wasmPath);

    // Check WASM magic number (0x00 0x61 0x73 0x6D)
    if (buffer[0] !== 0x00 || buffer[1] !== 0x61 || buffer[2] !== 0x73 || buffer[3] !== 0x6D) {
        throw new Error('Invalid WASM magic number');
    }

    // Check WASM version (1)
    if (buffer[4] !== 0x01 || buffer[5] !== 0x00 || buffer[6] !== 0x00 || buffer[7] !== 0x00) {
        throw new Error('Invalid WASM version');
    }

    return 'Valid WASM binary format';
});

// Test 6: WASM can be compiled
await asyncTest('WASM can be compiled', async () => {
    const buffer = fs.readFileSync(wasmPath);
    const module = await WebAssembly.compile(buffer);
    return 'WASM module compiled successfully';
});

// Test 7: WASM module information
await asyncTest('WASM module has exports', async () => {
    const buffer = fs.readFileSync(wasmPath);
    const module = await WebAssembly.compile(buffer);
    const exports = WebAssembly.Module.exports(module);
    const imports = WebAssembly.Module.imports(module);

    return `Exports: ${exports.length}, Imports: ${imports.length}`;
});

// Test 8: Estimate gzip size
console.log('\n📊 Size Analysis:');
test('Estimated gzip size', () => {
    const buffer = fs.readFileSync(wasmPath);
    const uncompressed = buffer.length;
    // Rough gzip estimation (typically 30-40% for WASM)
    const estimated = uncompressed * 0.35;
    return `${(uncompressed / 1024).toFixed(1)} KB → ~${(estimated / 1024).toFixed(1)} KB (gzip)`;
});

// Summary
console.log('\n' + '='.repeat(50));
const passed = testResults.filter(t => t.passed).length;
const total = testResults.length;
const passRate = ((passed / total) * 100).toFixed(1);

console.log(`\n📋 Test Summary:`);
console.log(`   Passed: ${passed}/${total} (${passRate}%)`);

if (passed === total) {
    console.log('\n✨ All tests passed! WASM build is ready.');
    process.exit(0);
} else {
    console.log('\n⚠️  Some tests failed. Please check the errors above.');
    process.exit(1);
}
