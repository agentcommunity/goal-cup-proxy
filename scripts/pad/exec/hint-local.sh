#!/usr/bin/env bash
# Pre script: Codex provider tables (one selected real host, one unselected local model) and a host list without it.
# PRD-11 acceptance 3 (PRD-12 section 3.1); pad-run passes $PAD_RUN/hosts.toml to goalcup start --hosts.
set -eu
cfg=$PAD_HOME/config.toml
{ echo 'model_provider = "x"'; cat "$cfg" 2>/dev/null || true; } > "$cfg.new"  # a top-level key goes before any table
mv "$cfg.new" "$cfg"
printf '%s\n' '' '[model_providers.x]' 'name = "x"' 'base_url = "https://api.openai.com/v1"' 'wire_api = "responses"' \
  'env_key = "OPENAI_API_KEY"' '' '[model_providers.local]' 'name = "local"' 'base_url = "http://127.0.0.1:11434/v1"' >> "$cfg"
echo "added model_provider = \"x\", [model_providers.x] and [model_providers.local] to $cfg"
sed -E 's/"api\.openai\.com"[[:space:]]*,?[[:space:]]*//g' "$PAD_MODEL_HOSTS" > "$PAD_RUN/hosts.toml"
echo "wrote $PAD_RUN/hosts.toml: $PAD_MODEL_HOSTS without api.openai.com"
