[English](CHANGELOG.md) | **Português**

# Changelog

Todas as mudanças relevantes deste projeto são documentadas aqui. O formato segue o [Keep a Changelog](https://keepachangelog.com/pt-BR/1.1.0/).

## [Não lançado]

### Adicionado

- Fase F0: bootstrap do workspace (dez crates, CI, cargo-deny, docs bilíngues).
- `tokred configure`: detecta a pasta do Claude Code, mostra um plano, faz o merge dos hooks do tokred no `settings.json` sem mexer em mais nada, instala seis subagentes, cria o banco SQLite e o `tokred.toml`.
- `tokred unconfigure [--purge]`: remove só o que o tokred adicionou.
- `tokred doctor [--json]`: informa hooks, agentes, detecção do rtk e problemas.
- `tokred hook <evento>`: stub fail-open para os seis hooks (o comportamento real chega nas fases F3 a F5).
