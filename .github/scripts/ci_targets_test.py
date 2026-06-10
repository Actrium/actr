#!/usr/bin/env python3
from __future__ import annotations

import unittest

import ci_targets


class CiTargetsTest(unittest.TestCase):
    def test_typescript_echo_workload_triggers_web_e2e(self) -> None:
        targets, reasons = ci_targets.detect_targets(
            ["examples/typescript/echo-workload/src/actr_service.ts"],
            full_run=False,
        )

        self.assertTrue(targets["ts_workload"])
        self.assertTrue(targets["web_binding"])
        self.assertIn(
            "typescript_workload_web_e2e:examples/typescript/echo-workload/src/actr_service.ts",
            reasons,
        )


if __name__ == "__main__":
    unittest.main()
