[English](CONTRIBUTING.md) | **Português**

# Contribuindo com o tokred

Obrigado por ajudar. Esta página explica como contribuir; leia também o [código de conduta](CODE_OF_CONDUCT.pt-BR.md). Para reportar uma vulnerabilidade, siga o [SECURITY.pt-BR.md](SECURITY.pt-BR.md) em vez de abrir uma issue pública.

## Maneiras de ajudar

- **Issues:** reporte um bug ou sugira uma melhoria com os modelos de issue. Diga o que você executou e em qual sistema operacional, e rode `tokred doctor --json` (remova qualquer informação privada antes de colar).
- **Documentação:** corrija um erro, esclareça uma página, melhore uma tradução.
- **Código:** escolha uma issue, ou abra uma para discutir a mudança antes de escrever uma grande.

## Configurar o ambiente

```sh
git clone https://github.com/sostenes198/tokred.git
cd tokred
cargo build
cargo test --workspace
cargo run -- doctor
```

Você precisa do Rust 1.89 ou mais novo. Mais em [desenvolvimento](docs/development.pt-BR.md).

## Fluxo de trabalho

1. Crie uma branch a partir da `main` (`feat/...`, `fix/...`, `docs/...`).
2. Faça commits pequenos e focados com [Conventional Commits](https://www.conventionalcommits.org/) **em inglês** (`feat:`, `fix:`, `docs:`, `chore:`...).
3. Abra um pull request contra a `main`. O CI precisa estar verde em Linux, macOS e Windows.

## Antes de abrir um pull request

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
```

- [ ] Comportamento novo tem testes; a correção de um bug inclui um teste que reproduz o bug.
- [ ] Nenhum teste toca o `~/.claude` real, a pasta de dados real nem a rede (use diretórios temporários, `TOKRED_DATA_DIR` e `--claude-dir`).
- [ ] A documentação está atualizada nos dois idiomas.
- [ ] `CHANGELOG.md` e `CHANGELOG.pt-BR.md` estão atualizados para mudanças visíveis ao usuário.

## Idioma

- **O código é em inglês:** nomes de arquivos, crates, módulos, tipos, funções e variáveis, tabelas e colunas do banco, chaves de configuração, comentários, mensagens de commit e todo texto que a CLI ou os hooks imprimem.
- **A documentação é bilíngue:** cada documento tem uma versão em inglês (`X.md`) e uma em português do Brasil (`X.pt-BR.md`) com o mesmo conteúdo. Se você não escreve português, abra o pull request com o texto em inglês e peça ajuda com a outra metade.

## Arquitetura

As regras de negócio são funções puras em `tokred-core` e nos crates de casos de uso; a E/S fica atrás das traits (portas) de `tokred-core` e só é composta em `tokred-cli`. Veja [arquitetura](docs/architecture.pt-BR.md) e [desenvolvimento](docs/development.pt-BR.md).

As decisões de projeto, e qualquer ponto em que a documentação oficial do Claude Code nos fez divergir de um plano, ficam registradas em [decisões](docs/decisions.pt-BR.md). Adicione uma entrada quando tomar uma.
