# DO NOT EDIT - Generated scaffold
# TODO: Implement your business logic

from __future__ import annotations

import argparse
import asyncio
import logging
import sys
from pathlib import Path

from actr import ActrNode

# Configure logging
logging.basicConfig(
    level=logging.INFO,
    format="[%(levelname)s] %(message)s",
)
logger = logging.getLogger(__name__)

# Add generated directory to Python path
generated_dir = Path(__file__).parent / "generated"
if str(generated_dir) not in sys.path:
    sys.path.insert(0, str(generated_dir))

async def main() -> int:
    ap = argparse.ArgumentParser(description="EchoService Runner")
    ap.add_argument("--actr-toml", required=True, help="ACTR config file path")
    args = ap.parse_args()

    logger.error(
        "Source-defined Python service workloads were removed. "
        "Build a verified .actr package and host it via Rust Hyper.attach_package(...)."
    )
    return 1


if __name__ == "__main__":
    try:
        sys_exit_code = asyncio.run(main())
        raise SystemExit(sys_exit_code)
    except KeyboardInterrupt:
        pass
