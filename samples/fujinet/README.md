# FujiNet HTTP fetch

[`http-fetch.act`](http-fetch.act) opens an HTTP resource, receives it in a
128-byte buffer, counts bytes through EOF and closes the channel. It shows
explicit channel settings and CASE handling for waiting, EOF, transport errors,
network errors and invalid helper inputs. It uses the embedded
[`ATARI.FUJINET.NET`](../../docs/ATARI_FUJINET_NET.md) module.

```sh
cargo run --bin actionc -- --mode optimized --runtime standalone samples/fujinet/http-fetch.act
cargo run --bin actionc -- --mode mir6502 --runtime standalone samples/fujinet/http-fetch.act
```

Both backends also support `--runtime cart`. Use a compatible Atari OS with a
configured, network-connected FujiNet or a verified software bridge. The pinned
protocol baseline is FujiNet v1.6.1. No CIO `N:` handler is needed. Ordinary disk
image support in an emulator is insufficient for network SIO commands.

Change the named `uri` string to a controlled HTTP resource larger than 128 bytes
when testing. `NET.HTTP_GET` selects direct GET, and the default translation is
binary. On EOF the example prints `Bytes received: <count>`. Failures are reported
before the separate Close result. It attempts at most 200 reads/status polls and
waits one video frame after WAITING; this is a polling bound, not a wall-clock
deadline. Larger resources need a larger bound and byte counter.

Build and VM protocol tests pass for modern classic/MIR6502 and both runtimes.
Live HTTP/TCP acceptance through FujiNet remains to be performed and recorded.
