# Build Notes

## open.mp component
- Default local build: `scripts/build-openmp-component.sh --openmp-root /path/to/open.mp`.
- Build and install into a server: `scripts/build-openmp-component.sh --openmp-root /path/to/open.mp --server-root /path/to/omp-server`.
- Clean rebuild: add `--clean`.

The open.mp checkout path is required through `OPEN_MP_ROOT` or `--openmp-root`.

Installed component artifacts go into `<server>/components`. Load order is `CEF`.

GitHub Actions builds the open.mp component next to the SA:MP server plugin.

## Cross-compiling the Windows client (macOS/Linux)
- Install `cargo-xwin`: `cargo install cargo-xwin --locked`.
- Ensure `XWIN_ACCEPT_LICENSE=1` is set (the script defaults it).
- Install `7z` (used to extract the DirectX SDK archive), `nasm`, and LLVM (for `llvm-lib`).
- Optionally set `DX_SDK` to an existing DirectX SDK `Lib/x86` directory; otherwise the script downloads and extracts it.
- Run `scripts/build-client-win32.sh`.

The script downloads `libcef.lib` if `CEF_PATH` is not set and builds `client`, `renderer`, and `loader` for `i686-pc-windows-msvc`. Outputs land in `target/i686-pc-windows-msvc/release/`.
