from __future__ import annotations

import argparse
import asyncio
import logging
from actr import ActrNode

APP_NAME = "EchoServer"

logging.basicConfig(level=logging.INFO, format="[%(levelname)s] %(message)s")
logger = logging.getLogger(__name__)

async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--actr-toml", required=True)
    args = ap.parse_args()

    logger.error(
        "%s source-defined workload entry was removed. "
        "Use a package-backed host with Rust Hyper.attach_package(...) instead.",
        APP_NAME,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
