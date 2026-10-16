# ADR 0049: Deliver a command's output as the guest produces it

## Status

Proposed.

## Context

The guest agent has always sent a command's output as authenticated chunks while the command runs.
Every layer above it collected those chunks and returned them when the command ended.
The runner's `exec/stream` route therefore sent its `stdout` and `stderr` events at the end, with comments in between, and said so: "when the facade gains incremental output, the same events arrive as the guest produces them".
A caller watching a long build or an interactive tool saw nothing until it finished.

## Decision

1. The guest-control host owner gains `execute_streaming`, which hands each authenticated chunk, already checked against the output allowance, to a callback.
   `execute` is the same loop with a callback that does nothing, so a streaming caller and a buffering caller observe one command and one set of accounting checks.
2. The vmm session and worker report each chunk as a response on the channel the final response uses, so a reply can never be attributed to the wrong command.
3. A jailed worker gets a request word of its own, `execute_stream`.
   It answers with any number of `chunk <stream> <hex>` packets and then the one final `executed` or `failure` reply, which is the only request with more than one reply.
   A worker that predates it refuses the word with one `rejected` packet before any chunk, which the supervisor reads as the command having been turned down.
4. A resident machine host gets a `ExecuteStreaming` call of its own, answered with `Chunk` lines and then one `StreamEnded` line that names the byte counts instead of repeating the bytes.
5. Every receiving side checks the chunks against the command's output allowance as they arrive and against the final byte counts, and treats a disagreement as an uncertain exchange: the handle is poisoned exactly as an uncertain buffered command poisons it.
6. `Backend::execute_streaming` has a default that runs the command to its end and replays the retained output in bounded chunks, so every backend answers a streaming request and only the KVM backend streams.
   `Engine::execute_machine_streaming` is `execute_machine` with the sink passed through, so admission, the receipt and the durable record are unchanged and a streamed command has the same idempotency.
   A replayed operation runs nothing and delivers no chunks.
7. The runner's `exec/stream` route forwards each chunk as an SSE event.
   Text events never cut a character in half: an incomplete trailing sequence is held until the next chunk completes it.
   A command with no output on a stream sends no event for that stream, where it used to send one empty event.
   A reader that leaves does not stop the command or its paperwork.

## Consequences

- Chunks reach the caller before the command has been validated and receipted, so a caller that must not act on unverified output waits for the final event.
- A slow reader applies backpressure to the guest exchange, bounded by the command's own deadline.
- The loopback `commands` route and the buffered runner `exec` route are unchanged.
- The control protocols grow by one request word and one call variant each, with older peers refusing them.
