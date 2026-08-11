# Preserved Action! Corresponding Source

`action-3.6-source-0b8bcedb.tar.gz` is a deterministic corresponding-source
snapshot for the bundled Action! 3.6 cartridge. It was produced from upstream
revision:

```text
repository: https://git.code.sf.net/p/atari-action/code
commit:     0b8bcedb9951acd84a1814d60e2eaeb0a93dd45f
```

Identity:

```text
size:    116822 bytes
SHA-256: fa3466ee7286d8e65a4ca5b0b1db69e4428b15ec93b2119ae68811a30528d824
```

The snapshot contains `JAC/src`, the upstream Action! build settings and build
script, and the reference ROM used by that script. It was generated with:

```sh
git archive --format=tar \
  --prefix=action-3.6-source-0b8bcedb/ \
  0b8bcedb9951acd84a1814d60e2eaeb0a93dd45f \
  JAC/src \
  JAC/build/Make-ACTION.bat \
  JAC/build/Make-Settings.bat \
  JAC/ref/rom/ACTION-36-ROM-OSS.rom \
  | gzip -n -9 > action-3.6-source-0b8bcedb.tar.gz
```

See `../ACTION-ROM-NOTICE.md` for license, provenance, and binary matching
details. The nightly publisher attaches this archive as a release asset.
