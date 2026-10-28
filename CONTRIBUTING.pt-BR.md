[English](CONTRIBUTING.md) | **Português**

# Contribuindo com o tokred

## Fluxo

1. Crie a branch a partir da `main` (`feat/...`, `fix/...`, `docs/...`).
2. Faça commits pequenos e focados, no padrão [Conventional Commits](https://www.conventionalcommits.org/) **em inglês** (`feat:`, `fix:`, `docs:`, `chore:`...).
3. Abra um pull request para a `main`; o CI precisa estar verde em Linux, macOS e Windows.

## Regras de idioma

- O código é em inglês: nomes de arquivos, crates, módulos, tipos, funções e variáveis, tabelas e colunas do banco, chaves da config, comentários, mensagens de commit e todas as strings exibidas pela CLI e pelos hooks.
- A documentação é **bilíngue**: cada documento tem a versão em inglês (`X.md`) e a versão em português (`X.pt-BR.md`) com o mesmo conteúdo. Atualize as duas no mesmo commit.

## Checklist antes de abrir o PR

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
```

- [ ] Todo comportamento novo tem testes; a correção de um bug inclui um teste que reproduz o bug.
- [ ] Nenhum teste toca o `~/.claude` real nem a pasta de dados real (use diretórios temporários, `TOKRED_DATA_DIR` e `--claude-dir`).
- [ ] Nenhum teste usa a rede.
- [ ] Docs atualizados nos dois idiomas.
- [ ] `CHANGELOG.md` e `CHANGELOG.pt-BR.md` atualizados para mudanças visíveis ao usuário.

## Regras de arquitetura

As regras de negócio são funções puras no `tokred-core` e nos crates de caso de uso; o I/O fica atrás dos traits (ports) do `tokred-core` e só é composto no `tokred-cli`. Veja [docs/architecture.pt-BR.md](docs/architecture.pt-BR.md) e [docs/development.pt-BR.md](docs/development.pt-BR.md).

Antes de implementar ou alterar um hook, consulte a documentação oficial de hooks (<https://code.claude.com/docs/en/hooks>) e registre qualquer divergência da spec em [docs/decisions.pt-BR.md](docs/decisions.pt-BR.md).
