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
| `TOKRED_PORT` | Porta do proxy para o `tokred proxy enable` e os scripts de instalação (1024 a 65535); a flag `--port` vale mais que ela |
| `TOKRED_RECORD_CONTENT` | Responde, sem perguntar, a pergunta do `tokred proxy enable` sobre gravar o conteúdo das conversas (`[proxy] log_bodies`): `true`, `false`, `yes`, `no`, `1` ou `0`; a flag `--record-content` vale mais que ela |
| `TOKRED_HOME` | Onde o script de instalação colocou o binário (`$TOKRED_HOME/bin`; padrão `~/.tokred`, Windows `%LOCALAPPDATA%\tokred`); o `tokred update` só substitui um binário que esteja ali |
| `TOKRED_RELEASE_BASE_URL` | Só nos scripts de instalação: pasta ou URL (`https://` ou `file://`) de onde baixar os assets da release (espelhos, instalação offline, testes) |
| `TOKRED_YES`, `TOKRED_NO_MODIFY_PATH`, `TOKRED_NO_ALIAS`, `TOKRED_NO_PROXY`, `TOKRED_NO_CONFIGURE`, `TOKRED_VERSION`, `TOKRED_PURGE` | Só nos scripts de instalação e desinstalação: a forma por variável de ambiente das opções deles, veja [instalação](installation.pt-BR.md) |
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
enabled = false                        # a chave do proxy (ligada pelo `tokred proxy enable`)
bind = "127.0.0.1:1029"                # endereço em que o proxy escuta
upstream = "https://api.anthropic.com" # único host para onde as requisições são encaminhadas
retention_days = 90                    # dias de registros do proxy e de turnos do painel guardados
log_bodies = false                     # guarda o conteúdo das mensagens para o painel (desligado: só metadados)
# ca_bundle = "/etc/ssl/corp-ca.pem"   # opcional (não é gravado por padrão): certificados de CA extras (PEM) em que confiar

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

O `tokred proxy enable` é o único comando que configura o proxy: grava `[proxy] bind`, `[proxy] log_bodies` e `[proxy] enabled = true` para você. Ele mantém o host de `bind` e troca só a porta; num terminal, pergunta a porta (padrão: a salva, senão 1029; a porta precisa estar entre 1024 e 65535 e livre, a menos que seja o proxy desta mesma instalação) e se o conteúdo das conversas pode ser gravado (padrão: o valor salvo por um `enable` anterior, senão sim). Sem terminal, ou com `--yes`, muda só o que uma flag ou variável de ambiente disser (`--port` / `TOKRED_PORT`, `--record-content` / `TOKRED_RECORD_CONTENT`) e deixa o resto como está; um primeiro `enable` sem respostas fica com a porta 1029 e sem gravar conteúdo. O `tokred configure` (hooks e agentes) não grava nenhum valor de `[proxy]` além dos padrões e nunca muda o que você escolheu. Veja [instalação](installation.pt-BR.md).

Os valores de `[modes]` passam a valer quando o hook correspondente ganhar o comportamento real (fases F3 a F5).

`ca_bundle` em `[proxy]` é opcional e não existe por padrão: o caminho de um arquivo PEM com autoridades certificadoras extras em que confiar para o upstream, além das raízes embutidas, do repositório do sistema operacional e dos arquivos indicados por `NODE_EXTRA_CA_CERTS` e `SSL_CERT_FILE`. O proxy também respeita `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` e `NO_PROXY` (e os nomes em minúsculas) do seu ambiente; veja [atrás de um proxy corporativo](proxy.pt-BR.md#atrás-de-um-proxy-corporativo).

O `[proxy] enabled` é a chave única dos recursos do proxy, `false` até você executar `tokred proxy enable` (o `tokred configure` não mexe nela: um valor que você escreveu é mantido, seja qual for, e o `unconfigure` não mexe no arquivo). Enquanto for `true`, o `tokred initialize` (executado pelo wrapper de shell do `claude` antes de cada abertura) sobe o proxy quando preciso e mantém o `env.ANTHROPIC_BASE_URL` do `settings.json` do Claude Code apontando para ele; enquanto for `false`, o tokred remove dali a sua própria URL, o `tokred claude` executa o `claude` direto, o `tokred proxy start` se recusa e o `tokred doctor` mostra "disabled". O `tokred proxy enable` e o `tokred proxy disable` o alteram e aplicam o efeito na hora. Um valor de `ANTHROPIC_BASE_URL` que não é do tokred (o seu próprio gateway) nunca é sobrescrito nem removido. A chave antiga `persist_base_url` não existe mais; um arquivo antigo que ainda a tenha continua funcionando (a chave é ignorada). Veja [proxy](proxy.pt-BR.md).

O `retention_days` vale para tudo o que o proxy guarda (requisições, decisões de roteamento, resumos de prompts, sugestões, medições shadow, resultados de ferramentas guardados e substituições da compressão): é apagado quando o proxy inicia e depois a cada hora. `0` guarda tudo. Os resultados guardados e as substituições de uma conversa que ainda teve requisição dentro do período são mantidos, seja qual for a idade. O `log_bodies` ainda não é lido.

As seções do proxy só importam quando o [proxy](proxy.pt-BR.md) está rodando. `tokred routing on|off` e `tokred compression on|off` definem o `mode` para você (reinicie o proxy para aplicar). `routing` e `compression` começam em `observe`: registram o que fariam e não mudam nada. O `[routing]` nunca altera o modelo do loop principal, seja qual for o valor; veja [roteamento](routing.pt-BR.md). `channel` em `[suggest]`: `stats` (padrão) as deixa disponíveis no `tokred suggest`, `statusline` inclui a status line, `message` também permite uma mensagem de uma linha em um momento seguro (veja [hooks](hooks.pt-BR.md)). As medições de `[shadow]` nunca alteram o tráfego.
