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
| `tokred-hooks` | Os seis handlers de hook, o guarda de tempo e os payloads do benchmark (fail-open) |
| `tokred-metrics` | Ingestão de transcripts, mesclagem do uso de transcripts e proxy, custo, economia, projeção, latência, seções do proxy e resumos de roteamento/guardas (puro) |
| `tokred-pricing` | Parse da lista de preços, snapshot embutido de fallback, casos de uso de sync e o adapter do LiteLLM |
| `tokred-update` | Detecção do método de instalação, verificação de versão, self-update com checksum e o adapter do GitHub |
| `tokred-audit` | Auditoria do contexto fixo: `CLAUDE.md` com imports, rules, skills, agentes, uso de MCP |
| `tokred-report` | O relatório HTML offline: modelo, gráficos SVG, template, gravação atômica |
| `tokred-store` | Adapter: SQLite (`rusqlite`), migrations versionadas, implementação dos repositórios |
| `tokred-proxy` | Adapter: o proxy local opcional (`hyper`, `tokio`, `rustls`): escuta no loopback, encaminha com streaming, mede uso e latência e executa o pipeline de hooks do proxy definido no `tokred-core`. **Único** crate que depende de `hyper` e `tokio`. Também serve o [painel](dashboard.pt-BR.md) ao vivo: uma visão em memória das sessões, um barramento de eventos (server-sent events) e o login, lendo o histórico pela porta `DashboardSource` do `tokred-core`, que o binário implementa sobre o `tokred-store` e o `tokred-report` |
| `tokred-routing` | Política de roteamento de modelo: classifica requisições de subagentes e auxiliares e escolhe um modelo mais barato quando é seguro. Puro; o modelo do loop principal nunca é alterado |
| `tokred-suggest` | Resumos dos prompts do loop principal e sugestões de um modelo mais barato. Puro; só aconselha |
| `tokred-compress` | Compressão reversível de resultados grandes de ferramentas e a lógica do servidor MCP `tokred_retrieve`. Depende só do `tokred-core` |
| `tokred` (`crates/tokred-cli`) | O binário. Faz o parse dos argumentos, monta os adapters reais, chama os casos de uso, formata a saída e decide o código de saída. Único crate que usa `anyhow` e `println!` |

Os quatro crates do proxy seguem a mesma direção do resto: `tokred-routing`, `tokred-suggest` e `tokred-compress` são puros e dependem só do `tokred-core` (encaixam-se no proxy pelo pipeline de hooks definido lá); `tokred-proxy` é o adapter que os executa sobre HTTP; o binário monta tudo e o executa com `tokred proxy start --foreground`. Veja [proxy](proxy.pt-BR.md).

Dentro do binário, o I/O de terminal passa por uma pequena abstração `Io` (flag interativa, entrada, saída) e a saída dos comandos é montada por funções puras, então prompts e relatórios são testados por unidade sem TTY.

O pacote do binário se chama `tokred`; o diretório mantém o nome `tokred-cli`.

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
