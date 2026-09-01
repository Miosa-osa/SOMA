# Command-line filesystem session, eval-1, 2026-09-01

Every file here was written by `proof-cli.sh` (retained one directory up), which ran sixteen
separate `soma` processes against one sandbox on the KVM Backend and the same prepared `node:22`
Generation the HTTP session used. The directory is a copy of `/srv/soma/guestfs/out-cli` on
eval-1.

For each numbered process:

  NN-name.command   the argument vector, as `soma ...`
  NN-name.stdout    standard output, which for `--format json` is the whole envelope
  NN-name.stderr    standard error
  NN-name.exit      the process exit status

`binary.bin` is the twelve bytes the host wrote. `binary-returned.bin` is the JSON envelope's
base64 `content` decoded back to bytes. `binary-human.bin` is a seventeenth invocation,
`--format human`, redirected straight to a file, which is the path a shell user would take.
`binary-verdict` compares all three.

The instance is 8ef411455c8c4cc49a849fcadab8f90d, launched by 01 and destroyed by 16.
