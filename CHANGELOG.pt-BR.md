[English](CHANGELOG.md) | **Português**

# Changelog

Todas as mudanças relevantes deste projeto são documentadas aqui. O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Não lançado]

## [0.1.0] - 2026-10-29

### Corrigido

- O `tokred uninstall` não se recusa mais a apagar uma pasta de dados que só tem os arquivos do proxy (ainda sem banco de dados nem `tokred.toml`).
- Mensagens mais claras: o `proxy enable` diz que o conteúdo das conversas "will not be recorded", a recusa do proxy a escutar em endereço fora do loopback não sugere mais um `--bind` que não pode permitir isso, e o `routing status --help` não promete mais um circuit breaker que não mostra.
- O `tokred uninstall` não executa mais a limpeza final a partir de um arquivo fixo na pasta temporária compartilhada: o script é gravado num arquivo novo, de nome único, que só o dono lê (outro usuário poderia ter plantado um arquivo no nome fixo, e dois desinstaladores ao mesmo tempo colidiam). A limpeza no Windows agora remove a entrada do PATH e o wrapper do perfil antes do binário, então a pasta do binário sumir significa que terminou.

## [0.1.0-rc.3] - 2026-10-29

### Corrigido

- Um proxy que ainda estava terminando quando o substituto iniciou apagava o arquivo de PID do substituto, então o `tokred proxy status` não mostrava o pid e o `tokred proxy enable` não achava o proxy em execução: desligar a gravação de conversas depois de um `stop`/`enable` ficava sem efeito. Agora o proxy só remove o arquivo de PID se ele ainda for o dele, e o `enable` também procura o proxy na porta configurada, aceitando só um que responda com o segredo desta instalação.

## [0.1.0-rc.2] - 2026-10-29

### Adicionado

- `tokred update --version 0.2.0-rc.3` instala exatamente essa versão (um release candidate ou uma mais antiga) em vez da última release estável, que nunca inclui pré-releases.

- O dashboard dá a cada sessão o nome que o Claude Code dá (o título que você escolheu ou o gerado, mais a pasta e um id curto), em vez de mostrar só o id da sessão.

### Corrigido

- O `tokred proxy enable --record-content false` agora vale para um proxy que já está rodando: o proxy lê essa configuração só ao iniciar, então continuava gravando o conteúdo das conversas até ser reiniciado. O `enable` agora pergunta ao proxy em execução o que ele grava (`content_recorded` no status dele) e o reinicia quando isso difere do pedido, ou quando ele é antigo demais para dizer.

- Os cartões do resumo do relatório de métricas (custo real, economias, tokens evitados, projeção, latência) agora acompanham os botões 7 dias / 30 dias / Tudo, com o período que cada um cobre; antes, só os gráficos diários mudavam. A barra diz "Summary and daily charts"; as seções de modelos, roteamento, guards, latência e auditoria continuam sobre o período todo.

- O `tokred update` diz que ainda não há release estável quando só existem pré-releases (o "latest" do GitHub responde 404 nesse caso), em vez de falhar com `http status: 404`.

## [0.1.0-rc.1] - 2026-10-29

### Adicionado

- Instalação com um comando: `install/install.sh` (macOS, Linux) e `install/install.ps1` (Windows) baixam o arquivo da release no GitHub, conferem o checksum, instalam em `~/.tokred/bin` (`%LOCALAPPDATA%\tokred\bin`), adicionam ao `PATH` e executam `tokred configure` e `tokred proxy enable`. `uninstall.sh` / `uninstall.ps1` desfazem. Opções: `--yes`, `--port`, `--version`, `--no-modify-path`, `--no-alias`, `--no-proxy`, `--no-configure`, e `TOKRED_HOME`, `TOKRED_RELEASE_BASE_URL`.
- O instalador define um wrapper `claude` (bash, zsh, fish, PowerShell) que executa `tokred initialize` primeiro e depois o Claude Code de verdade; fail-open, dispensado com `--no-alias` / `TOKRED_NO_ALIAS=1`, ignorado com `command claude`.
- O `tokred proxy enable` pergunta a porta do proxy (padrão 1029, de 1024 a 65535, precisa estar livre) e se o conteúdo das conversas pode ser gravado para o painel; novas flags `--port`, `--record-content <true|false>` e `--yes` e variáveis `TOKRED_PORT`, `TOKRED_RECORD_CONTENT` (a flag vale mais). Uma reexecução silenciosa nunca muda uma escolha sua, e um proxy em execução é reiniciado numa nova porta.
- `tokred uninstall`: para o proxy, remove o serviço de usuário dele, desliga a compressão, remove os hooks e agentes e, opcionalmente, a pasta de dados.
- Testes dos scripts de instalação (`tests/installer/`) no CI em Linux, macOS e Windows, e `shellcheck`.

### Alterado

- Não é publicado um build nativo para Windows em ARM: o crate `ring` não faz cross-compilação para `aarch64-pc-windows-msvc` no CI. O `install.ps1` instala o build x86_64 no Windows em ARM, que o executa pela emulação.

- O `tokred uninstall` remove o tokred por completo e não pergunta nada: o proxy e o serviço dele, os hooks, agentes e a URL do proxy no Claude Code, a pasta de dados e, depois, o binário, a entrada no `PATH` e o wrapper `claude` (o script de desinstalação vem embutido no binário). Se um passo falhar, o binário é mantido. `--yes` e `--purge` são aceitos e ignorados.
- O `tokred unconfigure` remove só os hooks e agentes; ele não toca mais no `ANTHROPIC_BASE_URL` do proxy (só `tokred proxy enable` / `disable` e `tokred uninstall` fazem isso).
- O `tokred configure` não configura mais o proxy: só mexe em hooks e agentes (mais o banco e o `tokred.toml` padrão) e não grava nenhum valor de `[proxy]` além dos padrões (`enabled = false`). O `tokred proxy enable` faz isso, com `--port` e `--record-content`; os scripts de instalação o executam depois do `configure`, e `--no-proxy` (`-NoProxy`, `TOKRED_NO_PROXY=1`) o pula. As flags `--proxy-port`, `--record-content` e `--no-record-content` do `configure` foram removidas, e um plano recusado sai com o código 2 (o script de instalação então também pula o proxy).
- O `tokred stats`, o `tokred metrics` e o relatório HTML leem os dados exatos por requisição do proxy como fonte primária e os transcripts para o que o proxy não viu, mesclados para que nenhuma requisição seja contada duas vezes ([metrics](docs/metrics.pt-BR.md)). Sem o proxy nada muda.
- Ao atualizar: instalações configuradas antes disto têm `enabled = false` gravado no `tokred.toml` e ele é mantido, então os recursos do proxy continuam desligados até você executar `tokred proxy enable` uma vez. Um serviço de usuário instalado deixa de iniciar o proxy enquanto ele está desabilitado (`tokred proxy service uninstall` o remove).
- A chave `persist_base_url` de `[proxy]` não existe mais (um arquivo antigo que ainda a tenha continua funcionando): o `enabled` agora faz o que ele deveria fazer.
- A distribuição é só a release do GitHub: os scripts de instalação e os arquivos compactados são assets da release. A publicação em Homebrew, Scoop, winget e crates.io, o `publish-package-managers.yml` e os segredos deles acabaram.
- O `tokred update` troca o binário só quando o script de instalação o colocou na pasta de instalação; qualquer outra cópia é orientada a reinstalar com o script. O aviso diário de versão segue a mesma regra.

- O `unconfigure` deixa intocado o que não entende nas configurações, em vez de falhar.

### Corrigido

- O `tokred unconfigure` agora registra que o tokred não está configurado, então a reaplicação que acompanha uma atualização não recoloca mais os hooks e agentes que você removeu (o `tokred configure` os liga de novo).

- `tokred uninstall` (e `tokred proxy status`) agora perguntam ao agendador do Windows se a tarefa de logon do proxy existe; a tarefa não tem arquivo, então era ignorada e ficava para trás.

- No Windows, o `tokred proxy start` executado com a saída capturada (`out=$(tokred proxy start)`) não espera mais o proxy terminar: o proxy em segundo plano é iniciado por `Start-Process`, que não repassa os pipes de quem o chamou.
- Gravações de cache com duração de 1 hora são precificadas pela própria tarifa (2x o input) em vez da tarifa de 5 minutos (1,25x); antes o `stats` as subestimava. O uso já registrado é corrigido automaticamente na próxima execução do `stats`.
- O "modelo mais recente de uma família" (usado na comparação tudo-no-opus e para modelos desconhecidos) agora é ordenado pela versão antes da data, então ids atuais sem data como `claude-opus-5-5` não perdem mais para ids 4.x datados.
- Fable e Mythos são reconhecidas como famílias de modelo (fallback de preço, cores no relatório).
- A ingestão de transcripts no Windows lia tamanhos de arquivo defasados enquanto o Claude Code ainda mantinha o transcript aberto.

- Comandos de hook são reconhecidos para qualquer executável cujo nome comece com `tokred` (binários renomeados ou com versão não geram mais entradas duplicadas).

### Adicionado

- O proxy funciona atrás de uma rede corporativa ([proxy](docs/proxy.pt-BR.md)): faz túnel com `CONNECT` por `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` (respeitando `NO_PROXY`) e confia nos certificados do sistema operacional e nos arquivos PEM de `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE` e da nova chave opcional `[proxy] ca_bundle`. Valores inválidos são ignorados com um aviso. O `tokred doctor` e o `tokred proxy status` mostram o host do proxy e os arquivos de CA.
- `tokred initialize`: faz o `settings.json` do Claude Code concordar com `[proxy] enabled`, para um wrapper de shell do `claude` executar antes de toda abertura. Habilitado: sobe o proxy se preciso e grava o `env.ANTHROPIC_BASE_URL` quando ele responder (removendo-o de novo, com um aviso, se o proxy não puder subir); desabilitado: remove a URL do tokred. Outro gateway nunca é sobrescrito. É silencioso, rápido (uma leitura de arquivo, uma sonda de saúde) e sempre sai com 0. `tokred proxy enable|disable` alternam a chave e aplicam na hora; `[proxy] enabled` agora é a chave única (com ela desligada, o `tokred claude` executa o `claude` direto, o `proxy start` se recusa e o `doctor` mostra "disabled"), o `configure` a liga em uma instalação nova e o `unconfigure` remove a URL do tokred. O hook `session-start` sobe um proxy morto quando a sessão aponta para ele, e o `doctor` avisa sobre uma URL que aponta para um proxy que não está rodando.
- Os registros de requisição do proxy agora incluem o motivo de parada, os nomes das ferramentas oferecidas, a duração de cache pedida (`5m`/`1h`) e o modo de roteamento em vigor (schema versão 6).
- O `[proxy] retention_days` é aplicado: dados mais antigos que isso são apagados quando o proxy inicia e a cada hora (`0` guarda tudo).
- Painel ao vivo servido pelo proxy: `GET /dashboard` mostra as sessões ativas e as últimas 10 e, de cada uma, o que foi enviado ao Claude, o que voltou (em streaming, conforme chega) e os subagentes em execução; `GET /metrics` serve o relatório HTML do `tokred metrics`. Tempo real por server-sent events; login com token de uso único (`tokred dashboard [--no-open]`). Veja [painel](docs/dashboard.pt-BR.md).
- `[proxy] log_bodies` e `[proxy] retention_days` agora são lidos: com `log_bodies = true` o painel também guarda o conteúdo das mensagens (só o que é novo em cada requisição e a resposta, truncados; nunca cabeçalhos nem credenciais), limpo para as sessões ativas e as últimas 10, e nunca mais antigo que `retention_days`. Desligado (o padrão), nada do conteúdo é lido nem guardado.
- Migration v5 do banco: `captured_turns` (motivo de parada e ferramentas de cada turno; conteúdo só com `log_bodies`).
- Proxy local opcional entre o Claude Code e a API da Anthropic, desligado até ser usado ([proxy](docs/proxy.pt-BR.md)): medição exata de cada requisição (modelo, tokens incluindo gravações de cache de 1 hora, latência, causa do cache miss, overhead do proxy); roteamento de requisições de subagentes e auxiliares para um modelo mais barato (`observe` por padrão; o modelo do loop principal nunca é alterado); conselhos sobre o loop principal (`tokred suggest`, `tokred statusline` e uma mensagem opcional de uma linha do hook `route`); compressão reversível de resultados grandes de ferramentas com o servidor MCP `tokred_retrieve` (`observe` por padrão). É fail-open: se algo falhar, a requisição original é encaminhada sem alteração.
- `tokred claude [args...]`: inicia o proxy se preciso e executa o Claude Code por ele só nesta execução, repassando todos os argumentos.
- `tokred proxy start|stop|status` e `tokred proxy service install|uninstall`; `tokred routing status|on|off|explain`; `tokred compression on|off|status` (`on` registra o servidor MCP em `~/.claude.json` com backup em `.tokred.bak`).
- O `tokred doctor` mostra o proxy (rodando, modos, para onde o `ANTHROPIC_BASE_URL` aponta); `TOKRED_BENCH_RUNS` define as execuções do `doctor --bench`.
- Com o proxy registrando: `tokred stats --by origin` e `--by agent-type`, e uma seção Proxy no `stats`, no `metrics` e no relatório HTML com o overhead contra o orçamento de 10 ms, as decisões de roteamento com economia observada e aplicada, o modelo pedido contra o que respondeu, os cache misses do prompt por causa e a última sugestão pendente (o `stats --json` ganha a chave `proxy`).
- Novas seções `[proxy]`, `[routing]`, `[routing.tiers]`, `[suggest]`, `[compression]` e `[shadow]` no `tokred.toml` ([configuração](docs/configuration.pt-BR.md)).
- Relatórios, preços, atualização e hooks: painéis coloridos no terminal (`tokred stats`, `stats latency`, `metrics`) com um relatório HTML offline; tabela de preços com snapshot embutido e sincronização diária; a verificação diária de versão e o `tokred update` (self-update com checksum verificado para instalações por script); `tokred audit`; `tokred cache`; os seis hooks (`guard-read`, `guard-search`, `route`, `enforce`, `session-check`, `session-start`) com registro de latência e `doctor --bench`; o pipeline de release com `dist` e publicação no Homebrew, Scoop, winget e crates.io.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL`, `TOKRED_RELEASES_URL`, `NO_COLOR` e `CLICOLOR_FORCE`.
- Bootstrap do workspace (dez crates, CI, cargo-deny, docs bilíngues).
- `tokred configure`: detecta a pasta do Claude Code, mostra um plano, faz o merge dos hooks do tokred no `settings.json` sem mexer em mais nada, instala seis subagentes, cria o banco SQLite e o `tokred.toml`.
- `tokred unconfigure [--purge]`: remove só o que o tokred adicionou.
- `tokred doctor [--json]`: informa hooks, agentes, outras ferramentas de hooks e problemas.
- `tokred hook <evento>`: stub fail-open para os seis hooks (o comportamento real chega em releases posteriores).
