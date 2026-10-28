[English](CHANGELOG.md) | **Português**

# Changelog

Todas as mudanças relevantes deste projeto são documentadas aqui. O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Não lançado]

### Alterado

- O `unconfigure` deixa intocado o que não entende nas configurações, em vez de falhar.

### Corrigido

- Comandos de hook são reconhecidos para qualquer executável cujo nome comece com `tokred` (binários renomeados ou com versão não geram mais entradas duplicadas).

### Adicionado

- Fases F1 a F5: painéis coloridos no terminal (`tokred stats`, `stats latency`, `metrics`) com um relatório HTML offline; tabela de preços com snapshot embutido e sincronização diária; a verificação diária de versão e o `tokred update` (self-update com checksum verificado para instalações por script); `tokred audit`; `tokred cache`; os seis hooks (`guard-read`, `guard-search`, `route`, `enforce`, `session-check`, `session-start`) com registro de latência e `doctor --bench`; o pipeline de release com `dist` e publicação no Homebrew, Scoop, winget e crates.io.
- `TOKRED_OFFLINE`, `TOKRED_PRICES_URL`, `TOKRED_RELEASES_URL`, `NO_COLOR` e `CLICOLOR_FORCE`.
- Fase F0: bootstrap do workspace (dez crates, CI, cargo-deny, docs bilíngues).
- `tokred configure`: detecta a pasta do Claude Code, mostra um plano, faz o merge dos hooks do tokred no `settings.json` sem mexer em mais nada, instala seis subagentes, cria o banco SQLite e o `tokred.toml`.
- `tokred unconfigure [--purge]`: remove só o que o tokred adicionou.
- `tokred doctor [--json]`: informa hooks, agentes, detecção do rtk e problemas.
- `tokred hook <evento>`: stub fail-open para os seis hooks (o comportamento real chega nas fases F3 a F5).
