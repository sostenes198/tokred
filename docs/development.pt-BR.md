[English](development.md) | **Português**

# Desenvolvimento

## Requisitos

- Rust stable (o MSRV do workspace é **1.89**, verificado no CI) com `rustfmt` e `clippy` (o `rust-toolchain.toml` instala).
- Para o gate local completo: `cargo install cargo-deny cargo-llvm-cov`.

## Build

```sh
cargo build                          # build debug de todos os crates
cargo run -- doctor                  # executa a CLI
cargo install --path crates/tokred-cli
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

Meta: pelo menos 80% no `tokred-core` e nos crates de caso de uso.

## Testando um hook localmente

Hooks leem JSON do stdin, então dá para rodar um à mão contra uma sandbox:

```sh
export TOKRED_DATA_DIR=$(mktemp -d)
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"Cargo.lock"}}' \
  | cargo run -q -- hook guard-read; echo "exit=$?"
```

Antes de implementar um hook, leia a documentação oficial (<https://code.claude.com/docs/en/hooks>) para o formato exato de entrada e saída, e registre qualquer divergência da spec em [decisões](decisions.pt-BR.md).

## Releases

O pipeline de release (`dist`, tap do Homebrew, bucket do scoop, winget, `cargo publish`) faz parte da fase F1. Nota para essa fase: o `tokred-install` embute `templates/agents/*.md` com `include_str!` a partir da raiz do repositório, então esses arquivos precisam ser empacotados com o crate para o `cargo publish` funcionar.
