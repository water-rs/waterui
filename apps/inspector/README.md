# WaterUI Inspector

The Inspector is a standalone WaterUI application that attaches to a running
application's inspector endpoint: it negotiates the protocol, renders the view
tree, and streams element snapshots and profiling data back to the developer.

`water inspector` prepares and launches it — see `cli/README.md`.

## Release package

The expected release-asset contract — production is **not wired into the
release pipeline yet** — is one asset per supported host triple on the
framework's `v<x.y.z>` GitHub release, named
`waterui-inspector-<rust-triple>.zip`. The archive holds a single
`waterui-inspector/` directory:

```
waterui-inspector/
  waterui-inspector        # the client executable (waterui-inspector.exe on Windows)
  .cache.json              # written by the CLI after verification, not part of the archive
  resources/
    fonts/                 # the app's declared bundled fonts
    waterui_assets/        # the app's asset root
  <native runtime libraries>  # e.g. dxc on Windows
```

The release record's per-asset `digest` field must carry
`sha256:<64-hex>` of the archive — the CLI downloads over HTTPS, verifies the
bytes against that digest, extracts, runs `waterui-inspector --identity`
(10s bound, no credentials in the environment) and caches the package under
`~/.water/inspector/releases/<version>/<triple>/` only when the reported
identity matches the identity the endpoint answered `Identify` with —
framework version, protocol version and, for a checkout build, the source
revision.

A release archive is built with `WATERUI_INSPECTOR_PROTOCOL_BUILD_MODE=release`
so the protocol's build script stamps no checkout commit into
`InspectorBuildIdentity`; a checkout build resolves `HEAD` itself.
