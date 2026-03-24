async function main() {
  throw new Error(
    [
      'actr-ts no longer supports source-defined local workloads.',
      'Use a package-backed host via Rust Hyper.attach_package(...) instead.',
    ].join(' '),
  );
}

main().catch(console.error);
