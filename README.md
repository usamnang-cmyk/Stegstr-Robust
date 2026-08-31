# Stegstr Robust Edition

Freelancer contest entry **#93** — a dual-mode Stegstr CLI whose Robust JPEG path is built to survive WhatsApp / Telegram / Instagram **photo** processing.

Official Stegstr embeds only in PNG (DWT). JPEG, resize, or recompression corrupts that payload. This repo closes that gap with **Block Average QIM + repetition + CRC**.

## What judges should run

```bash
git clone https://github.com/usamnang-cmyk/Stegstr-Robust.git
cd Stegstr-Robust/src-tauri
cargo build --release --bin stegstr-cli

curl -L -o cover.jpg "https://picsum.photos/1600/1200"

./target/release/stegstr-cli embed cover.jpg \
  -o stego-robust.jpg \
  --payload "Contest test message – Stegstr Robust Edition" \
  --mode robust \
  --platform whatsapp

./target/release/stegstr-cli test-survival stego-robust.jpg \
  --simulate whatsapp \
  --output after-whatsapp.jpg

./target/release/stegstr-cli detect after-whatsapp.jpg
```

Expected detect output:

```
Contest test message – Stegstr Robust Edition
```

## Commands

| Command | Purpose |
|---|---|
| `embed` | Hide a string or `@file` in an image |
| `detect` | Extract + verify (`--json` for agents) |
| `decode` | Raw extract |
| `post` | Write a Nostr-style `{version, events}` bundle |
| `prepare` | Resize/quality-target a carrier |
| `test-survival` | Simulate WhatsApp / Telegram / Instagram recompress |
| `capacity` | Estimate usable payload bytes |

```bash
./target/release/stegstr-cli post "Hello from Robust Stegstr" --output bundle.json
./target/release/stegstr-cli embed cover.jpg -o stego.jpg --payload @bundle.json --mode robust --encrypt
./target/release/stegstr-cli detect stego.jpg --json
./target/release/stegstr-cli capacity cover.jpg --mode robust
```

## Modes

| Mode | Redundancy | Typical payload | Photo-send survival |
|---|---|---|---|
| `robust` (Fortress) | r=15 BA-QIM | 20–30+ bytes on ~2MP | Designed for WhatsApp standard |
| `armor-max` | r=8 | higher | Usually, with prep |
| `armor-standard` | r=4 | higher | Milder platforms |
| `lossless` | PNG LSB | KB range | No — file/document only |

## Layout

```
Stegstr-Robust/
  README.md
  agents.txt
  LICENSE
  docs/          proposal + notes
  schema/        bundle schema
  src-tauri/     Rust CLI (stegstr-cli)
```

## Notes for the contest owner

- Stay on Freelancer for judging questions.
- Real-world test: send `stego-robust.jpg` as a **photo** (not a document) on WhatsApp / Telegram / Instagram, download the received file, then `detect` it.
- `--encrypt` uses the documented open-mode derived key so any Stegstr Robust build can detect it. It is not a substitute for Nostr NIP-04 DMs.
