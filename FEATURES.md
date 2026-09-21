# Feature matrix

Every message of the Stratum V2 subprotocols the harness targets, and how far the harness
takes it. The columns are the stages a message goes through: whether a program can express it
(IR), whether the compiler lowers it (compiler), whether the runner sends or receives it
(runner), whether an oracle judges it (oracle), and whether a test exercises it (test).
`recv` in the runner column means the dispatcher decodes and classifies the message when a
server sends it; `send` means a program can send it.

Direction is from the harness's point of view: it plays a downstream client, so `->` is sent
by the harness and `<-` is received from the role under test.

The scope today is the `SetupConnection` flow. Every other message is decoded when a server
sends it and kept in the trace as a message of a type the harness does not model, and none
can be sent except as raw bytes.

## Common protocol (extension 0)

| message | direction | IR | compiler | runner | oracle | test |
| --- | --- | --- | --- | --- | --- | --- |
| SetupConnection | -> | yes | yes | send | SetupConnectionOracle | oracle, pool |
| SetupConnection.Success | <- | success block | yes | recv | version, flags, protocol | oracle, pool |
| SetupConnection.Error | <- | skips the block | yes | recv | flags reporting | oracle, pool |
| ChannelEndpointChanged | <- | no | no | recv as other | no | no |
| Reconnect | <- | no | no | recv | no | no |
| unknown message type | -> | SendRawFrame | yes | send, recv as unknown | no | generators |
| unknown extension | -> | SendRawFrame | yes | send, recv as unknown | no | generators |

## Mining, template distribution and job declaration protocols

Not implemented. A session can be set up for any of the three, and the success block that
follows is typed for the protocol, but no message of any subprotocol can be sent inside it
yet. What a server sends of them is decoded and kept in the trace as other.
