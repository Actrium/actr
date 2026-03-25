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
    ap.add_argument("message_count", type=int, help="Number of stream messages to receive")
    args = ap.parse_args()
    
    logger.error(
        "Python stream-echo client relied on a local source workload. "
        "That path was removed; use a package-backed host/client flow instead."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(asyncio.run(main()))
