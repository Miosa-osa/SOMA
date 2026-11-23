"""Measure seeded CartPole rollouts with synchronous or process-based vector environments."""

import argparse
import json
import time
from contextlib import closing
from functools import partial
from statistics import mean, median

import gymnasium as gym
import numpy as np
from gymnasium.vector import AsyncVectorEnv, AutoresetMode, SyncVectorEnv


def collect(args):
    factories = [partial(gym.make, "CartPole-v1")] * args.workers
    cls = SyncVectorEnv if args.mode == "sync" else AsyncVectorEnv
    options = {"context": "spawn"} if args.mode == "async" else {}
    rng = np.random.default_rng(args.seed)
    started = time.perf_counter()
    with closing(cls(factories, autoreset_mode=AutoresetMode.SAME_STEP, **options)) as envs:
        observations, _ = envs.reset(seed=args.seed)
        startup_seconds = time.perf_counter() - started
        completed = np.zeros(args.workers, dtype=np.int64)
        running_returns = np.zeros(args.workers, dtype=np.float64)
        returns = []
        terminal_count = 0
        time_limit_count = 0
        simulator_steps = 0
        started = time.perf_counter()

        while (active := completed < args.episodes_per_worker).any():
            if args.policy == "balance":
                # Push toward the pole's estimated angle in 0.1 seconds.
                actions = (observations[:, 2] + 0.1 * observations[:, 3] > 0).astype(np.int64)
            else:
                actions = rng.integers(0, 2, size=args.workers)

            observations, rewards, terminated, truncated, _ = envs.step(actions)
            simulator_steps += args.workers
            running_returns[active] += rewards[active]
            finished = active & (terminated | truncated)
            returns.extend(running_returns[finished].tolist())
            terminal_count += int(terminated[finished].sum())
            time_limit_count += int((truncated & ~terminated)[finished].sum())
            completed[finished] += 1
            running_returns[finished] = 0

        elapsed = time.perf_counter() - started

    return {
        "environment": "CartPole-v1",
        "mode": args.mode,
        "policy": args.policy,
        "seed": args.seed,
        "workers": args.workers,
        "episodes_per_worker": completed.tolist(),
        "completed_episodes": len(returns),
        "terminated_episodes": terminal_count,
        "time_limit_episodes": time_limit_count,
        "mean_return": mean(returns),
        "median_return": median(returns),
        # Default CartPole pays one point per step, including the final step.
        "recorded_steps": int(sum(returns)),
        "simulator_steps": simulator_steps,
        "startup_seconds": startup_seconds,
        "rollout_seconds": elapsed,
        "simulator_steps_per_second": simulator_steps / elapsed,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--mode", choices=("sync", "async"), default="sync")
    parser.add_argument("--policy", choices=("random", "balance"), default="balance")
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--workers", type=int, default=4)
    parser.add_argument("--episodes-per-worker", type=int, default=25)
    args = parser.parse_args()
    if args.seed < 0 or not 1 <= args.workers <= 32 or args.episodes_per_worker < 1:
        parser.error("use a nonnegative seed, 1 to 32 workers, and a positive episode count")
    print(json.dumps(collect(args), allow_nan=False))


if __name__ == "__main__":
    main()
