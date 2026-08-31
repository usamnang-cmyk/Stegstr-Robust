# Judge path (copy-paste)

```bash
cd src-tauri
cargo build --release --bin stegstr-cli
curl -L -o cover.jpg "https://picsum.photos/1600/1200"
./target/release/stegstr-cli embed cover.jpg -o stego-robust.jpg --payload "Contest test message – Stegstr Robust Edition" --mode robust --platform whatsapp
./target/release/stegstr-cli test-survival stego-robust.jpg --simulate whatsapp --output after-whatsapp.jpg
./target/release/stegstr-cli detect after-whatsapp.jpg
```

Checklist

- [ ] CLI builds from source
- [ ] Robust embed + detect on a clean image
- [ ] Payload survives test-survival
- [ ] CLI is non-interactive and supports --json
- [ ] Errors are explicit
