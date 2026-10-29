[English](CHANGELOG.md) | **Português**

# Changelog

Todas as mudanças relevantes deste projeto são documentadas aqui. O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Não lançado]

### Alterado

- O `unconfigure` deixa intocado o que não entende nas configurações, em vez de falhar.

### Corrigido

- No Windows, o `tokred proxy start` executado com a saída capturada (`out=$(tokred proxy start)`) não espera mais o proxy terminar: o proxy em segundo plano é iniciado por `Start-Process`, que não repassa os pipes de quem o chamou.
- Gravações de cache com duração de 1 hora são precificadas pela própria tarifa (2x o input) em vez da tarifa de 5 minutos (1,25x); antes o `stats` as subestimava. O uso já registrado é corrigido automaticamente na próxima execução do `stats`.
- O "modelo mais recente de uma família" (usado na comparação tudo-no-opus e para modelos desconhecidos) agora é ordenado pela versão antes da data, então ids atuais sem data como `claude-opus-5-5` não perdem mais para ids 4.x datados.
- Fable e Mythos são reconhecidas como famílias de modelo (fallback de preço, cores no relatório).
- A ingestão de transcripts no Windows lia tamanhos de arquivo defasados enquanto o Claude Code ainda mantinha o transcript aberto.

- Comandos de hook são reconhecidos para qualquer executável cujo nome comece com `tokred` (binários renomeados ou com versão não geram mais entradas duplicadas).

### Adicionado

- O proxy funciona atrás de uma rede corporativa ([proxy](docs/proxy.pt-BR.md#atrás-de-um-proxy-corporativo)): faz túnel com `CONNECT` por `HTTPS_PROXY` / `HTTP_PROXY` / `ALL_PROXY` (respeitando `NO_PROXY`) e confia nos certificados do sistema operacional e nos arquivos PEM de `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE` e da nova chave opcional `[proxy] ca_bundle`. Valores inválidos são ignorados com um aviso. O `tokred doctor` e o `tokred proxy status` mostram o host do proxy e os arquivos de CA.
- Proxy local opcional entre o Claude Code e a API da Anthropic, desligado até ser usado ([proxy](docs/proxy.pt-BR.md)): medição exata de cada requisição (modelo, tokens incluindo gravações de cache de 1 hora, latência, causa do cache miss, overhead do proxy); roteamento de requisições de subagentes e auxiliares para um modelo mais barato (`observe` por padrão; o modelo do loop principal nunca é alterado); conselhos sobre o loop principal (`tokred suggest`, `tokred statusline` e uma mensagem opcional de uma linha do hook `route`); compressão reversível de resultados grandes de ferramentas com o servidor MCP `tokred_retrieve` (`observe` por padrão). É fail-open: se algo falhar, a requisição original é encaminhada sem alteração.
- `tokred claude [args...]`: inicia o proxy se preciso e executa o Claude Code por ele só nesta execução, repassando todos os argumentos.
- `tokred proxy start|stop|status` e `tokred proxy service install|uninstall`; `tokred routing status|on|off|explain`; `tokred compression on|off|status` (`on` registra o servidor MCP em `~/.claude.json` com backup em `.tokred.bak`).
- O `tokred doctor` mostra o proxy (rodando, modos, para onde o `ANTHROPIC_BASE_URL` aponta); `TOKRED_BENCH_RUNS` define as execuções do `doctor --bench`.
- Novas seções `[proxy]`, `[routing]`, `[routing.tiers]`, `[suggest]`, `[compression]` e `[shadow]` no `tokred.toml` ([configuração](docs/configuration.pt-BR.md)).
- Fases F1 a F5: painéis coloridos no terminal (`tokred stats`, `stats latency`, `metrics`) com um relatório HTML offline; tabela de preços com snapshot embutido e sincronização diária; a verificação diária de versão e o `tokred update` (self-update com checksum verificado para instalações por script); `tokred audit`; `tokred cache`; os seis hooks (`guard-read`, `guard-search`, `route`, `enforce`, `session-check`, `session-start`) com registro de latência e `doctor --bench`; o pipeline de release com `dist` e publicação no Homebrew, Scoop, winget e crates.io.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL`, `TOKRED_RELEASES_URL`, `NO_COLOR` e `CLICOLOR_FORCE`.
- Fase F0: bootstrap do workspace (dez crates, CI, cargo-deny, docs bilíngues).
- `tokred configure`: detecta a pasta do Claude Code, mostra um plano, faz o merge dos hooks do tokred no `settings.json` sem mexer em mais nada, instala seis subagentes, cria o banco SQLite e o `tokred.toml`.
- `tokred unconfigure [--purge]`: remove só o que o tokred adicionou.
- `tokred doctor [--json]`: informa hooks, agentes, detecção do rtk e problemas.
- `tokred hook <evento>`: stub fail-open para os seis hooks (o comportamento real chega nas fases F3 a F5).
