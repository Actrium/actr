import importlib.util
import json
import tempfile
import unittest
from pathlib import Path


SCRIPT_PATH = Path(__file__).with_name("coverage_summary.py")


def load_coverage_summary_module():
    spec = importlib.util.spec_from_file_location("coverage_summary", SCRIPT_PATH)
    module = importlib.util.module_from_spec(spec)
    assert spec.loader is not None
    spec.loader.exec_module(module)
    return module


class CoverageSummaryTest(unittest.TestCase):
    def test_merges_lcov_and_kover_line_totals(self):
        coverage_summary = load_coverage_summary_module()

        with tempfile.TemporaryDirectory() as tmp_dir:
            tmp = Path(tmp_dir)
            rust_lcov = tmp / "rust.lcov"
            web_rust_lcov = tmp / "web-rust.lcov"
            ts_lcov = tmp / "typescript.lcov"
            kotlin_xml = tmp / "kotlin.xml"
            output_dir = tmp / "summary"

            rust_lcov.write_text(
                "TN:\nSF:src/lib.rs\nLF:10\nLH:7\nend_of_record\n",
                encoding="utf-8",
            )
            web_rust_lcov.write_text(
                "TN:\nSF:bindings/web/crates/common/src/lib.rs\nLF:15\nLH:6\nend_of_record\n",
                encoding="utf-8",
            )
            ts_lcov.write_text(
                "TN:\nSF:index.ts\nLF:5\nLH:5\nend_of_record\n",
                encoding="utf-8",
            )
            kotlin_xml.write_text(
                """
                <report name="actr-kotlin">
                  <counter type="LINE" missed="2" covered="8" />
                </report>
                """,
                encoding="utf-8",
            )

            coverage_summary.main(
                [
                    "--rust-lcov",
                    str(rust_lcov),
                    "--web-rust-lcov",
                    str(web_rust_lcov),
                    "--typescript-lcov",
                    str(ts_lcov),
                    "--kotlin-xml",
                    str(kotlin_xml),
                    "--output-dir",
                    str(output_dir),
                ]
            )

            data = json.loads((output_dir / "coverage-summary.json").read_text())

            self.assertEqual(data["aggregate"]["covered_lines"], 26)
            self.assertEqual(data["aggregate"]["total_lines"], 40)
            self.assertEqual(data["aggregate"]["line_coverage"], 65.0)
            self.assertEqual(
                [language["language"] for language in data["languages"]],
                ["Rust", "Web Rust (host unit)", "TypeScript", "Kotlin"],
            )


if __name__ == "__main__":
    unittest.main()
