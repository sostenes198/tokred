[English](configuration.md) | **Português**

# Configuração

## Locais

| Item | Origem |
|---|---|
| Pasta do Claude Code | `--claude-dir`, depois `[claude] dir` no `tokred.toml`, depois `$CLAUDE_CONFIG_DIR`, depois `~/.claude` (Windows: `%USERPROFILE%\.claude`) |
| Pasta de dados do tokred | `$TOKRED_DATA_DIR` se definido; senão Linux `~/.local/share/tokred`, macOS `~/Library/Application Support/tokred`, Windows `%APPDATA%\tokred` |
| Banco | `<dados>/tokred.db` |
| Config | `<dados>/tokred.toml` |
| Logs | `<dados>/logs/` (rotação diária, mantém 7 dias) |
| Relatórios | `<dados>/reports/metrics.html` |

## Variáveis de ambiente

| Variável | Efeito |
|---|---|
| `TOKRED_DATA_DIR` | Substitui a pasta de dados (também usada pelos testes) |
| `CLAUDE_CONFIG_DIR` | Pasta do Claude Code, como o próprio Claude Code a lê |
| `TOKRED_LOG` | Nível de log gravado no arquivo: `error`, `warn`, `info` (padrão), `debug`, `trace` |
| `TOKRED_NO_UPDATE_CHECK` | `1` desliga a verificação diária de versão |
| `TOKRED_OFFLINE` | `1` desliga toda chamada de rede (verificação de versão, sincronização de preços, `update`, `pricing sync`) |
| `TOKRED_PRICES_URL` | URL alternativa da lista de preços do LiteLLM (espelhos, testes) |
| `TOKRED_RELEASES_URL` | URL alternativa da API de "última release" (espelhos, testes) |
| `NO_COLOR` / `CLICOLOR_FORCE` | Desligam as cores do terminal / as forçam |

## `tokred.toml`

Criado pelo `configure` com todas as chaves abaixo. Na atualização, as chaves novas entram com o valor padrão e **os valores existentes são preservados**; o arquivo só é regravado quando faltava alguma chave (assim seus comentários sobrevivem nos demais casos).

```toml
[claude]
dir = "/Users/voce/.claude"   # gravado pelo `tokred configure`

[modes]
route = "warn"          # off | warn | enforce
enforce = "warn"
guard_read = "enforce"
guard_search = "warn"
session_check = "warn"

[guard_read]
max_read_bytes = 204800
deny_globs = ["**/package-lock.json", "**/pnpm-lock.yaml", "**/yarn.lock",
              "**/Cargo.lock", "**/poetry.lock", "**/*.min.js", "**/*.map",
              "**/dist/**", "**/build/**", "**/node_modules/**", "**/target/**", "**/.git/**"]

[guard_search]
large_repo_files = 5000

[session_check]
context_warn_tokens = 120000

[update]
mode = "notify"         # off | notify | auto

[pricing]
auto_sync = true

[cache]
l1_ttl = "4h"
l2_ttl = "24h"
l2_hits = 3
l3_hits = 10
```

Os valores de `[modes]` passam a valer quando o hook correspondente ganhar o comportamento real (fases F3 a F5).
