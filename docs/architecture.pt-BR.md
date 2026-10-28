[English](architecture.md) | **Português**

# Arquitetura

Arquitetura limpa no estilo idiomático de Rust: **regras de negócio puras no centro, I/O nas bordas**, dependências apontando sempre para dentro e um único ponto de composição (o binário).

```
tokred (cli) ──► casos de uso (install, hooks, metrics, pricing, audit, update, report) ──► tokred-core
     │                                                                                          ▲
     └──► adapters (tokred-store, módulos fs/http) ── implementam os traits (ports) de ─────────┘
```

## Crates

| Crate | Papel |
|---|---|
| `tokred-core` | Domínio: tipos, erros, modelo de config, regras de paths, modelo dos eventos de hook e os ports (`Clock`, `FileSystem`, `ManifestRepo`; outros chegam com suas fases). **Sem I/O.** |
| `tokred-install` | Casos de uso `configure`, `unconfigure`, `doctor`: merge puro do settings, templates dos agentes e regras de instalação. Contém o adapter de `std::fs` (módulo `fs`) |
| `tokred-hooks` | Handlers dos hooks (stubs fail-open na F0) |
| `tokred-metrics`, `tokred-pricing`, `tokred-audit`, `tokred-update`, `tokred-report` | Casos de uso das fases seguintes (placeholders na F0) |
| `tokred-store` | Adapter: SQLite (`rusqlite`), migrations versionadas, implementação dos repositórios |
| `tokred` (`crates/tokred-cli`) | O binário. Faz o parse dos argumentos, monta os adapters reais, chama os casos de uso, formata a saída e decide o código de saída. Único crate que usa `anyhow` e `println!` |

O pacote do binário se chama `tokred` para que `cargo install tokred` funcione; o diretório mantém o nome `tokred-cli`.

## Regras

- Os casos de uso dependem só do `tokred-core`. A lógica principal é feita de **funções puras** sobre dados (`merge_settings`, `remove_tokred`, `plan_agent`, `merge_config`…); o I/O entra por traits recebidos por parâmetro.
- Nenhum caso de uso importa `rusqlite`, `ureq` nem chama `std::fs` diretamente; os adapters fazem isso.
- Erros são enums `thiserror` por crate; `anyhow` só no binário.
- Lints: `unsafe_code = forbid`; clippy `pedantic` como warning; `unwrap`, `expect`, `panic`, `todo`, `dbg!` negados fora de testes.
- O `Clock` existe para que TTLs, a verificação de versão de 24 h e as métricas por dia sejam testáveis sem esperar.

## Fluxo de um hook

1. O Claude Code executa `"<path>/tokred" hook <evento>` e escreve JSON no stdin.
2. O `main` trata `hook` **antes** do parse de argumentos, então até um evento desconhecido sai com código 0.
3. O handler (`tokred-hooks`) roda dentro de `catch_unwind`; em erro ou panic ele loga em `<dados>/logs/` e não produz saída.
4. Se houver uma decisão, ela é impressa como JSON no stdout (`hookSpecificOutput`, `permissionDecision`, `additionalContext`, `systemMessage`); o processo sai com código 0.

## Fluxo do `configure`

`escolhe a pasta do Claude → acha o path estável do binário → plan_configure (só leitura) → mostra o plano → confirma → cria pasta de dados/config/banco → apply_configure (backup, gravação atômica do settings.json, agentes, manifesto)`.
