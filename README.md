# ffxiv-act-native

Build FFXIV ACT plugins as Rust `cdylib` libraries. The crate generates a small
AnyCPU .NET Framework shim from `Advanced Combat Tracker.exe` and the official
`FFXIV_ACT_Plugin.Common.dll`; it does not read or reference
`FFXIV_ACT_Plugin.dll`.

## Documentation

- [Getting started](docs/index.md) — create, build, and install a plugin
- [Reference](docs/reference.md) — UI, shim generation, runtime rules, and testing
- [Runnable example](examples/ffxiv-plugin) — a minimal plugin with native UI

## License

[BlueOak-1.0.0](LICENSE.md)
