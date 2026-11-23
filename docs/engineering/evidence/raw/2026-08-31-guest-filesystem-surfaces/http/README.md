# HTTP filesystem session, eval-1, 2026-08-31

Every file here was written by `proof-http.sh` (retained one directory up) against `soma-api`
bound to `127.0.0.1:18901`, backed by the KVM Backend and the prepared `node:22` Generation at
one vCPU, 1024 MiB of memory and 10240 MiB of storage. The directory is a copy of
`/srv/soma/guestfs/out-http` on eval-1, minus the server's own log.

For each numbered exchange:

  NN-name.request   the method, URL, headers and body exactly as they were sent
  NN-name.response  the response body exactly as it came back
  NN-name.status    the HTTP status code

`binary.bin` is the twelve bytes the host wrote, `binary-returned.bin` is the read response's
base64 `content` decoded back to bytes, and `binary-verdict` is the comparison of the two.

The instance is 50d2b493a9f54bdb94e9925b80e9c88d, created by 01-create and destroyed by 16.
