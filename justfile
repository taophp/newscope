set dotenv-load

@default:
  just --list

@start:
  OLLAMA_API_KEY="dummy" cargo run --bin newscope
