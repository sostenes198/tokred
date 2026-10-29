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
| `TOKRED_BENCH_RUNS` | Execuções por payload do `tokred doctor --bench` (padrão 50) |
| `TOKRED_HOOK_GUARD_MS` | Substitui o limite de 2 s de um hook (usado pelos testes) |
| `TOKRED_PROXY_OVERHEAD_REPORT` | Só para o teste de overhead do proxy: arquivo onde gravar os percentis medidos |
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

[proxy]
enabled = false                        # reservado: ainda não é lido
bind = "127.0.0.1:1029"                # endereço em que o proxy escuta
upstream = "https://api.anthropic.com" # único host para onde as requisições são encaminhadas
persist_base_url = false               # reservado: ainda não é lido
retention_days = 90                    # reservado: ainda não é lido
log_bodies = false                     # reservado: ainda não é lido

[routing]
mode = "observe"                       # off | observe | enforce
min_confidence = 0.8                   # confiança mínima da classificação (0..1) para agir
min_saving_usd = 0.005                 # economia mínima estimada de uma decisão
max_rewrite_failure_rate = 0.05        # taxa de reescritas rejeitadas que abre o circuit breaker
cooldown = "30m"                       # por quanto tempo o circuit breaker fica aberto
allowed_models = []                    # vazio: qualquer modelo mais barato do catálogo
compaction = "keep"                    # keep | cheap (opt-in)

[routing.tiers]                        # classe da tarefa -> cheap | mid | keep
search = "cheap"
summary = "cheap"
implementation = "mid"
planning = "keep"
architecture = "keep"
review = "keep"
auxiliary = "cheap"

[suggest]
channel = "stats"                      # off | stats | statusline | message
min_simple_prompts = 3                 # prompts simples seguidos antes de uma sugestão
cooldown = "6h"                        # tempo mínimo entre duas sugestões em uma sessão

[compression]
mode = "observe"                       # off | observe | enforce
min_tokens = 1500                      # só resultados de ferramentas acima disto (bytes / 4) são trocados
preview_head_lines = 40
preview_tail_lines = 20
retention = "7d"                       # por quanto tempo os originais são guardados para o `tokred_retrieve`
max_retrievals = 3                     # leituras de volta de um id antes de excluir sua ferramenta

[shadow]
compression = true                     # mede o que a compressão economizaria
response_cache = true                  # conta requisições idênticas repetidas
```

Os valores de `[modes]` passam a valer quando o hook correspondente ganhar o comportamento real (fases F3 a F5).

As seções do proxy só importam quando o [proxy](proxy.pt-BR.md) está rodando. `tokred routing on|off` e `tokred compression on|off` definem o `mode` para você (reinicie o proxy para aplicar). `routing` e `compression` começam em `observe`: registram o que fariam e não mudam nada. O `[routing]` nunca altera o modelo do loop principal, seja qual for o valor; veja [roteamento](routing.pt-BR.md). `channel` em `[suggest]`: `stats` (padrão) as deixa disponíveis no `tokred suggest`, `statusline` inclui a status line, `message` também permite uma mensagem de uma linha em um momento seguro (veja [hooks](hooks.pt-BR.md)). As medições de `[shadow]` nunca alteram o tráfego.
