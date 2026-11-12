# Vendored crates

Copies of two [iced](https://github.com/iced-rs/iced) 0.14 crates (MIT, see each `LICENSE`),
used through `[patch.crates-io]` in `Cargo.toml`. Each has a few lines changed, marked
`Truss patch`, so that transparent windows (Settings > Appearance > Transparency) work on Windows:

- `iced_wgpu` 0.14.0: reads wgpu's backend options from the environment (so DX12 can present
  through a DirectComposition visual) and prefers pre-multiplied alpha, which DXGI composition
  swapchains require.
- `iced_winit` 0.14.1: creates a transparent window without a GDI redirection surface.
