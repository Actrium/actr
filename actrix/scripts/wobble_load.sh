#!/usr/bin/env bash
# Generates gentle CPU load fluctuation for testing gauge animation.
# Spawns/kills stress workers in a sine-like pattern.
# Usage: ./scripts/wobble_load.sh [duration_seconds]
# Ctrl-C to stop early.

DURATION=${1:-120}
CORES=$(nproc)
WORKERS=$(( CORES > 2 ? CORES / 2 : 1 ))

echo "Wobbling load for ${DURATION}s using up to ${WORKERS} worker(s) on ${CORES} core(s). Ctrl-C to stop."

pids=()

cleanup() {
  echo "Stopping..."
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null
  done
  wait 2>/dev/null
  echo "Done."
  exit 0
}
trap cleanup INT TERM

end=$(( $(date +%s) + DURATION ))

while [ "$(date +%s)" -lt "$end" ]; do
  # Busy phase: 3-6s
  busy=$(( RANDOM % 4 + 3 ))
  for (( i=0; i<WORKERS; i++ )); do
    ( t=$(date +%s%N); while :; do x=$(( t * t )); done ) &
    pids+=($!)
  done
  sleep "$busy"

  # Kill workers
  for pid in "${pids[@]}"; do
    kill "$pid" 2>/dev/null
  done
  wait 2>/dev/null
  pids=()

  # Idle phase: 4-8s
  idle=$(( RANDOM % 5 + 4 ))
  sleep "$idle"
done

cleanup
