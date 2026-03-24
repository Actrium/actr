from __future__ import annotations

import argparse
import asyncio
import logging

from actr import ActrNode

logging.basicConfig(
    level=logging.INFO,
    format="[%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

async def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--actr-toml", required=True)
    args = ap.parse_args()
    
    logger.error(
        "Python stream-echo server relied on a local source workload. "
        "That path was removed; host the service from a verified .actr package instead."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
