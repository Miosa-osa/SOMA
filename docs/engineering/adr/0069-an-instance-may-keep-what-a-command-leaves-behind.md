# 0069. An Instance may keep what a command leaves behind

By default nothing survives a command.
The agent kills the command's process group and then sweeps every other process except the ones named in `/run/soma/survivors`.

A sandbox needs the opposite for one common case.
A user starts a development server in the background with one command and opens its port with the next.
On a Firecracker sandbox the server keeps running; on SOMA the sweep ends it when its command returns.

An Instance now states an exec policy in `/run/soma/exec-policy`.
A line `keep-session` keeps what a command leaves behind when the command ends on its own.
The agent then neither kills the command's process group nor sweeps it, and spares that group until its last member exits.
It forgets a group that has no member left, so a later process that reuses the identifier is never spared by mistake.

A command that reaches its deadline or its output limit, or whose output cannot be delivered, still ends with everything it started.
Shutdown and the pre-capture sweep are unchanged and end every process.

The policy belongs to the Instance, not to the Generation.
One Generation serves Instances that keep leftovers and Instances that end them.
A missing, empty or unreadable file is the default, so an Instance whose controller never writes the file behaves exactly as before.
The file lives in `/run`, which every Instance gets fresh, so a clone never inherits a policy from the machine that was captured.

Writing the file needs no privilege a command does not already have: any command can already name its own session in the survivor list.
