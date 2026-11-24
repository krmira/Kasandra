# Kasandra

Tiny multi-provider chat client built with Rust + egui. One source file (`src/main.rs`).

- Providers: OpenAI-compatible (OpenAI, Groq, OpenRouter, xAI, Cerebras, custom), Anthropic, Gemini
- Add a key via the model button next to the chat bar, then press the green check
- Custom endpoint: paste `https://host/v1 <key>`

## Run
    cargo run --release

## Release
    git tag v0.1.0 && git push --tags

GitHub Actions (`.github/workflows/build.yml`) builds Windows, Linux and macOS.
