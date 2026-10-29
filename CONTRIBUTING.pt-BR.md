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
```

Você precisa do Rust 1.89 ou mais novo. Na nova pasta, compile com `cargo build`, rode os testes com `cargo test --workspace` e experimente o seu build com `cargo run -- doctor`. Mais em [desenvolvimento](docs/development.pt-BR.md).

## Fluxo de trabalho

1. Crie uma branch a partir da `main` (`feat/...`, `fix/...`, `docs/...`).
2. Faça commits pequenos e focados com [Conventional Commits](https://www.conventionalcommits.org/) **em inglês** (`feat:`, `fix:`, `docs:`, `chore:`...).
3. Abra um pull request contra a `main`. O CI precisa passar: ele roda os testes em Linux, macOS e Windows, além das outras verificações abaixo.

## Antes de abrir um pull request

Rode a verificação de qualidade descrita em [desenvolvimento](docs/development.pt-BR.md): formatação, Clippy, testes, a geração da documentação e `cargo deny check`. O CI roda as mesmas verificações. Depois, confira esta lista:

- [ ] Comportamento novo tem testes; a correção de um bug inclui um teste que reproduz o bug.
- [ ] Nenhum teste toca o `~/.claude` real, a pasta de dados real nem a rede (use diretórios temporários, `TOKRED_DATA_DIR` e `--claude-dir`).
- [ ] A documentação está atualizada nos dois idiomas, e `python3 scripts/check-docs.py` imprime `documentation ok`.
- [ ] `CHANGELOG.md` e `CHANGELOG.pt-BR.md` estão atualizados para mudanças visíveis ao usuário.

## Idioma

- **O código é em inglês:** nomes de arquivos, crates, módulos, tipos, funções e variáveis, tabelas e colunas do banco, chaves de configuração, comentários, mensagens de commit e todo texto que a CLI ou os hooks imprimem.
- **A documentação é bilíngue:** cada documento tem uma versão em inglês (`X.md`) e uma em português do Brasil (`X.pt-BR.md`) com o mesmo conteúdo. Se você não escreve português, abra o pull request com o texto em inglês e peça ajuda com a outra metade.

## Arquitetura

As regras de negócio são funções puras em `tokred-core` e nos crates de casos de uso; a E/S fica atrás de traits (portas) e só é composta em `tokred-cli`. Veja [arquitetura](docs/architecture.pt-BR.md) e [desenvolvimento](docs/development.pt-BR.md).

As decisões de projeto e seus motivos ficam registrados em [decisões](docs/decisions.pt-BR.md). Adicione uma entrada quando tomar uma.
