[English](development.md) | **Português**

# Desenvolvimento

## Requisitos

- Rust stable (o MSRV do workspace é **1.89**, verificado no CI) com `rustfmt` e `clippy` (o `rust-toolchain.toml` instala).
- Para o gate local completo: `cargo install cargo-deny cargo-llvm-cov`.

## Build

```sh
cargo build                          # build debug de todos os crates
cargo run -- doctor                  # executa a CLI
```

## Verificações (as mesmas do CI)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
cargo +1.89 check --workspace --all-targets --locked    # MSRV
```

## Testes

| Tipo | Onde | O que cobre |
|---|---|---|
| Unitário | `#[cfg(test)]` dentro de cada módulo | Funções puras: merge do settings, decisão de instalação dos agentes, merge da config, parsing do nome do hook |
| Integração | `crates/<crate>/tests/` | `tokred-store` num SQLite real; `tokred-install` configure/unconfigure/doctor contra fixtures em diretórios temporários reais |
| Ponta a ponta | `crates/tokred-cli/tests/` | O binário real via `assert_cmd`: `configure` → hooks → `doctor` → `unconfigure`, em sandbox |
| Snapshot | junto dos anteriores (`insta`) | `settings.json` resultante do merge, hooks do `doctor --json` |
| Propriedade | `crates/tokred-install/tests/merge_props.rs` (`proptest`) | configure duas vezes = uma; configure + unconfigure = original; hooks de terceiros sobrevivem |

Regras: nenhum teste toca o `~/.claude` real nem a pasta de dados real, e nenhum teste usa a rede. Os testes ponta a ponta definem `HOME`, `CLAUDE_CONFIG_DIR` e `TOKRED_DATA_DIR` como diretórios temporários (um deles com espaço no nome). Toda correção de bug entra com um teste que reproduz o bug.

Os arquivos de teste de integração começam com `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]` porque as opções `allow-*-in-tests` do clippy só cobrem código `#[cfg(test)]`, não arquivos em `tests/`.

### Snapshots

```sh
INSTA_UPDATE=always cargo test        # grava snapshots novos/alterados
git diff -- '*.snap'                  # revise antes de commitar
```

### Fixtures

`tests/fixtures/` tem variações de `settings.json` (vazio, com hooks do rtk, com hooks do usuário, com versão antiga do tokred, JSON inválido, chaves desconhecidas). Adicione uma fixture sempre que encontrar um formato real que o merge precise tratar.

## Cobertura

```sh
cargo llvm-cov --workspace --all-features --summary-only
```

Meta: pelo menos 80% no `tokred-core` e nos crates de caso de uso; o workspace está hoje em **100% das linhas** (toda linha instrumentada executa) e 100% das funções.

Para listar linhas que nunca executam:

```sh
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info   # DA:<linha>,0 = nunca executada
cargo llvm-cov report --text                                               # código anotado, `^0` marca uma região que nunca rodou
```

A cobertura de regiões é menor (cerca de 98%) porque cada `?` tem um ramo de erro; forçar cada falha de I/O só traria ruído, não segurança. Prompts, o texto do plano e a saída do `doctor` são testados por unidade através da abstração `Io` e de funções puras de renderização, então nenhum teste precisa de terminal.

## Testando um hook localmente

Hooks leem JSON do stdin, então dá para rodar um à mão contra uma sandbox. Um hook não faz nada se a pasta de dados e o banco ainda não existirem (uma instalação apagada com `--purge` tem de continuar apagada), então configure antes uma sandbox descartável:

```sh
export TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
mkdir "$CLAUDE_CONFIG_DIR/projects"
cargo run -q -- configure --yes
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"Cargo.lock"}}' \
  | cargo run -q -- hook guard-read; echo "exit=$?"
```

`TOKRED_BENCH_RUNS=3 cargo run -q -- doctor --bench` roda o benchmark de latência com menos execuções (o padrão é 50 por payload).

Antes de implementar um hook, leia a documentação oficial (<https://code.claude.com/docs/en/hooks>) para o formato exato de entrada e saída, e registre qualquer divergência da spec em [decisões](decisions.pt-BR.md).

## Releases

As releases são construídas pelo `dist` (`dist-workspace.toml`, `.github/workflows/release.yml` gerado) quando uma tag `vX.Y.Z` é enviada; o workflow envia para a release do GitHub os arquivos, seus `.sha256` e os scripts de instalação (`install/*.sh`, `install/*.ps1`), e mais nada (sem gerenciadores de pacotes, sem segredos). O passo a passo completo (cada release, ensaio, reversão) está em [release](release.pt-BR.md); rodar e testar à mão está em [testes](testing.pt-BR.md). Veja também [instalação](installation.pt-BR.md). Depois de mudar o `dist-workspace.toml`, rode `dist generate` e comite o resultado; `dist plan` mostra o que uma release conteria. Os templates dos agentes ficam dentro do crate `tokred-install` (`crates/tokred-install/templates/`).

Os scripts de instalação são `sh` POSIX e PowerShell simples, finos de propósito: todo prompt (porta, gravação de conteúdo, confirmações) vive em Rust (`tokred configure`, `tokred uninstall`), onde é testado por unidade através da costura `Io`. Mudou um script? Rode `sh tests/installer/install_test.sh`.
