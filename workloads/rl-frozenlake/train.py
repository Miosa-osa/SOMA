"""Train a tabular policy in a headless Gymnasium environment."""

import argparse
import json
import random

import gymnasium as gym


def greedy_action(values):
    return max(range(len(values)), key=values.__getitem__)


def train(environment, rng, episodes, seed):
    values = [
        [0.0] * environment.action_space.n for _ in range(environment.observation_space.n)
    ]
    for episode in range(episodes):
        state, _ = environment.reset(seed=seed + episode)
        explore = max(0.1, 1 - episode / episodes)
        while True:
            action = (
                rng.randrange(environment.action_space.n)
                if rng.random() < explore
                else greedy_action(values[state])
            )
            next_state, reward, terminated, truncated, _ = environment.step(action)
            future_value = 0.0 if terminated else max(values[next_state])
            target = reward + 0.95 * future_value
            values[state][action] += 0.5 * (target - values[state][action])
            if terminated or truncated:
                break
            state = next_state
    return [greedy_action(row) for row in values]


def evaluate(environment, action_for, episodes, seed):
    goals = 0
    for episode in range(episodes):
        state, _ = environment.reset(seed=seed + episode)
        while True:
            state, reward, terminated, truncated, _ = environment.step(action_for(state))
            if terminated or truncated:
                goals += int(reward > 0)
                break
    return goals


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--train-episodes", type=int, default=2000)
    parser.add_argument("--eval-episodes", type=int, default=100)
    args = parser.parse_args()
    if args.seed < 0 or min(args.train_episodes, args.eval_episodes) < 1:
        parser.error("the seed must be nonnegative and episode counts must be positive")

    environment = gym.make("FrozenLake-v1", map_name="4x4", is_slippery=False)
    eval_seed = args.seed + args.train_episodes
    try:
        policy = train(environment, random.Random(args.seed), args.train_episodes, args.seed)
        random_policy = random.Random(eval_seed)
        baseline = evaluate(
            environment,
            lambda _: random_policy.randrange(environment.action_space.n),
            args.eval_episodes,
            eval_seed,
        )
        trained = evaluate(environment, policy.__getitem__, args.eval_episodes, eval_seed)
    finally:
        environment.close()

    print(json.dumps({
        "environment": "FrozenLake-v1",
        "map": "4x4",
        "slippery": False,
        "seed": args.seed,
        "training_episodes": args.train_episodes,
        "evaluation_episodes": args.eval_episodes,
        "random_goal_count": baseline,
        "trained_goal_count": trained,
        "policy": policy,
    }, allow_nan=False))


if __name__ == "__main__":
    main()
