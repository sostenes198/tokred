[English](configuration.md) | **Português**

# Configuração

Onde o tokred guarda seus arquivos, quais variáveis de ambiente ele lê e todas as chaves do `tokred.toml` com seus padrões.

## Locais

| Item | Local |
|---|---|
| Pasta do Claude Code | `--claude-dir`, depois `[claude] dir` no `tokred.toml`, depois `$CLAUDE_CONFIG_DIR`, depois `~/.claude` (Windows: `%USERPROFILE%\.claude`) |
| Pasta de dados do tokred | `$TOKRED_DATA_DIR` se definida; senão Linux `~/.local/share/tokred`, macOS `~/Library/Application Support/tokred`, Windows `%APPDATA%\tokred` |
| Banco de dados | `<data>/tokred.db` |
| Config | `<data>/tokred.toml` |
| Logs | `<data>/logs/` (um arquivo por dia, 7 dias mantidos) |
| Relatório HTML | `<data>/reports/metrics.html` |
| Arquivos de execução do proxy | `<data>/proxy/` (arquivo de pid, segredo de administração) |

## Variáveis de ambiente

| Variável | Efeito |
|---|---|
| `TOKRED_DATA_DIR` | Substitui a pasta de dados |
| `CLAUDE_CONFIG_DIR` | Pasta do Claude Code, como o próprio Claude Code a lê |
| `TOKRED_LOG` | Nível gravado no arquivo de log: `error`, `warn`, `info` (padrão), `debug`, `trace` |
| `TOKRED_OFFLINE` | `1` desliga toda chamada de rede: verificação de versão, sincronização de preços, `update`, `pricing sync` |
| `TOKRED_NO_UPDATE_CHECK` | `1` desliga só a verificação diária de versão |
| `TOKRED_PORT` | Porta do proxy para `tokred proxy enable` e o script de instalação (1024 a 65535); a flag `--port` vence |
| `TOKRED_RECORD_CONTENT` | Responde, sem perguntar, à pergunta do `tokred proxy enable` sobre registrar o conteúdo das conversas (`true`, `false`, `yes`, `no`, `1`, `0`); a flag `--record-content` vence |
| `TOKRED_BENCH_RUNS` | Execuções por entrada do `tokred doctor --bench` (padrão 50) |
| `TOKRED_PRICES_URL` | URL alternativa da lista de preços do LiteLLM (espelhos) |
| `TOKRED_RELEASES_URL` | URL alternativa da API de "última release" (espelhos) |
| `TOKRED_HOOK_GUARD_MS` | Substitui o limite de 2 s de um hook, em milissegundos (feito para testes) |
| `TOKRED_HOME` | Onde o script de instalação colocou o binário (`$TOKRED_HOME/bin`; padrão `~/.tokred`, Windows `%LOCALAPPDATA%\tokred`). O `tokred update` só troca um binário que esteja lá |
| `NO_COLOR` | Qualquer valor não vazio desliga as cores do terminal |
| `CLICOLOR_FORCE` | Qualquer valor não vazio diferente de `0` força as cores, mesmo quando o stdout não é um terminal |

Os scripts de instalação e desinstalação também leem `TOKRED_VERSION`, `TOKRED_YES`, `TOKRED_NO_MODIFY_PATH`, `TOKRED_NO_ALIAS`, `TOKRED_NO_PROXY`, `TOKRED_NO_CONFIGURE` e `TOKRED_RELEASE_BASE_URL`; eles estão descritos em [instalação](installation.pt-BR.md). O proxy lê as variáveis de rede usuais (`HTTPS_PROXY`, `NO_PROXY`, `NODE_EXTRA_CA_CERTS`, ...), descritas em [proxy](proxy.pt-BR.md).

## tokred.toml

O `tokred configure` cria o arquivo com todas as chaves no valor padrão. O arquivo que ele grava não tem comentários e suas chaves ficam em ordem alfabética; a ordem e os comentários abaixo servem só para leitura. Edite-o com qualquer editor; reiniciar o proxy aplica as mudanças de `[proxy]`, `[routing]`, `[suggest]`, `[compression]` e `[shadow]`, e os hooks leem o arquivo a cada execução.

**Como as atualizações fazem o merge.** No `configure` e depois de uma atualização de versão, as chaves que faltam são adicionadas com o padrão e todo valor que você definiu é mantido. O arquivo só é reescrito quando faltava uma chave, então os seus comentários sobrevivem nos outros casos. Uma chave que o tokred não conhece mais é ignorada. Um `tokred.toml` que não pode ser lido faz os hooks usarem os padrões.

```toml
[claude]
dir = "/Users/you/.claude"   # salvo pelo `tokred configure`

[modes]                      # off | warn | enforce, um por hook
route = "warn"
enforce = "warn"
guard_read = "enforce"
guard_search = "warn"
session_check = "warn"

[guard_read]
max_read_bytes = 204800      # arquivos maiores exigem offset/limit
deny_globs = ["**/package-lock.json", "**/pnpm-lock.yaml", "**/yarn.lock",
              "**/Cargo.lock", "**/poetry.lock", "**/*.min.js", "**/*.map",
              "**/dist/**", "**/build/**", "**/node_modules/**", "**/target/**", "**/.git/**"]

[guard_search]
large_repo_files = 5000      # um repositório com mais arquivos conta como grande

[session_check]
context_warn_tokens = 120000 # avisa o usuário quando o contexto passa disto

[update]
mode = "notify"              # off | notify | auto (auto hoje se comporta como notify)

[pricing]
auto_sync = true             # atualiza a tabela de preços no máximo uma vez por dia

[cache]
l1_ttl = "4h"                # cache em camadas, veja architecture.pt-BR.md
l2_ttl = "24h"
l2_hits = 3                  # acertos necessários para chegar ao L2
l3_hits = 10                 # acertos necessários para chegar ao L3

[proxy]
enabled = false                        # a chave geral dos recursos do proxy
bind = "127.0.0.1:1029"                # endereço em que o proxy escuta
upstream = "https://api.anthropic.com" # o único host para o qual as requisições são encaminhadas
retention_days = 90                    # dias de registros do proxy e de turnos capturados do painel mantidos; 0 mantém tudo
log_bodies = false                     # registra o conteúdo das conversas para o painel (desligado: só metadados)
# ca_bundle = "/etc/ssl/corp-ca.pem"   # opcional, ausente por padrão: certificados de CA extras (PEM) em que confiar

[routing]
mode = "observe"                       # off | observe | enforce
min_confidence = 0.8                   # confiança mínima da classificação (0..1) para agir
min_saving_usd = 0.005                 # economia estimada mínima de uma decisão
max_rewrite_failure_rate = 0.05        # taxa de reescritas rejeitadas que abre o disjuntor
cooldown = "30m"                       # por quanto tempo o disjuntor fica aberto
allowed_models = []                    # vazio: qualquer modelo mais barato do catálogo
compaction = "keep"                    # keep | cheap

[routing.tiers]                        # classe de tarefa -> cheap | mid | keep
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
retention = "7d"                       # por quanto tempo os originais ficam guardados para o `tokred_retrieve`
max_retrievals = 3                     # leituras de volta de um id antes de a ferramenta ser excluída

[shadow]                               # medições que nunca alteram o tráfego
compression = true                     # mede quanto a compressão economizaria
response_cache = true                  # conta requisições repetidas exatas
```

### Notas por seção

| Seção | O que saber |
|---|---|
| `[modes]` | O que cada modo faz está em [hooks e agentes](hooks-and-agents.pt-BR.md). `route` e `session_check` nunca negam, então `enforce` se comporta como `warn` para eles. |
| `[guard_read]`, `[guard_search]`, `[session_check]` | Limites dos hooks de mesmo nome. Os `deny_globs` são comparados com o caminho relativo à pasta da sessão. |
| `[update]` | `off` nunca verifica novas versões; `notify` imprime um aviso (no máximo uma requisição por dia). `auto` é aceito e se comporta como `notify`: nada se atualiza sozinho, rode `tokred update`. |
| `[pricing]` | Com `auto_sync = false`, os preços só mudam com `tokred pricing sync`. |
| `[cache]` | As durações aceitam `s`, `m`, `h` ou `d` (`90s`, `30m`, `4h`, `7d`). |
| `[proxy]` | O `tokred proxy enable` grava `enabled`, `bind` (muda a porta e mantém o host) e `log_bodies` por você; o `configure` nunca os altera. Com `enabled = false`, o tokred remove o próprio `ANTHROPIC_BASE_URL` das configurações do Claude Code, `tokred claude` executa o `claude` diretamente, `proxy start` se recusa e o `doctor` mostra `disabled`. `retention_days` vale para tudo que o proxy guarda (requisições, decisões de roteamento, resumos, sugestões, medições, resultados de ferramentas guardados) e roda quando o proxy inicia e depois a cada hora. Com `log_bodies = false` só metadados são registrados, nunca o texto das mensagens. `ca_bundle` se soma às raízes embutidas, ao repositório do sistema e aos arquivos indicados por `NODE_EXTRA_CA_CERTS` e `SSL_CERT_FILE`. Veja [proxy](proxy.pt-BR.md). |
| `[routing]`, `[routing.tiers]` | Nunca mudam o modelo do loop principal, diga o que disserem. `observe` registra o que o roteamento faria e não muda nada. `tokred routing on` / `off` definem `mode` para você. Veja [roteamento](routing.pt-BR.md). |
| `[suggest]` | `off` impede que sugestões sejam produzidas. Qualquer outro valor as deixa disponíveis em `tokred suggest` e, se você definir `tokred statusline` como sua barra de status, nela também. `message` também deixa uma mensagem de uma linha chegar até você em um momento seguro (veja [hooks e agentes](hooks-and-agents.pt-BR.md)). Precisa do proxy. |
| `[compression]` | `tokred compression on` / `off` definem `mode` para você. Com `observe` (o padrão) o tokred apenas mede. |
| `[shadow]` | Medições para o relatório; nunca alteram o tráfego. |
