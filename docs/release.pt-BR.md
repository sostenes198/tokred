[English](release.md) | **Português**

# Rodar localmente e publicar uma versão

Duas partes: [rodar o tokred na sua máquina](#1-rodar-localmente) e depois [entregar um binário que as pessoas instalam com um comando](#2-publicar-uma-versão). As verificações de quem contribui (testes, cobertura, lints) estão em [desenvolvimento](development.pt-BR.md).

## 1. Rodar localmente

### Compilar e rodar a partir do código-fonte

```sh
cargo build                       # build de debug
cargo run -q -- doctor            # rode qualquer comando: cargo run -q -- <comando>
cargo build --release             # binário otimizado em target/release/tokred
```

Requisitos: Rust stable (MSRV 1.89; o `rust-toolchain.toml` instala `rustfmt` e `clippy`). Um binário compilado assim funciona de onde estiver, mas o `tokred update` manda reinstalar com o script (só um binário na pasta de instalação se atualiza sozinho).

### Experimentar sem mexer no seu ambiente real

Use uma sandbox descartável para que nada seja gravado no seu `~/.claude` nem na sua pasta de dados reais:

```sh
export TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
mkdir "$CLAUDE_CONFIG_DIR/projects"
cargo run -q -- configure --yes   # hooks, agentes e o banco vão para a sandbox
cargo run -q -- doctor            # confira o resultado
cargo run -q -- uninstall --yes --purge
```

`TOKRED_OFFLINE=1` desliga a verificação de versão e a sincronização de preços.

### Experimentar os scripts de instalação sem uma release

Os scripts baixam de `TOKRED_RELEASE_BASE_URL`, que aceita uma pasta `file://`. Monte o arquivo do jeito que o `dist` organiza e instale a partir dele numa home descartável:

```sh
cargo build
sh tests/installer/install_test.sh      # faz exatamente isso, numa sandbox, e confere o resultado
```

À mão: coloque `tokred-<target>/tokred` em `tokred-<target>.tar.gz`, escreva `<sha256>  tokred-<target>.tar.gz` em `tokred-<target>.tar.gz.sha256` e então

```sh
HOME=$(mktemp -d) TOKRED_HOME=$(mktemp -d) TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) \
  TOKRED_RELEASE_BASE_URL=file:///caminho/da/pasta sh install/install.sh --yes --port 23456
```

No Windows, `tests\installer\install_test.ps1` faz o mesmo (ele edita o `PATH` do usuário e o perfil do PowerShell e os restaura; fora do CI ele exige `-AllowRealProfile`).

### Rodar o proxy

O [proxy](proxy.pt-BR.md) é opcional. Com o binário de verdade (não `cargo run`, para o processo em segundo plano sobreviver ao build):

```sh
tokred proxy enable               # uma vez: escolhe a porta e a gravação de conteúdo, liga os recursos e sobe o proxy
tokred proxy start                # em segundo plano; ou `tokred proxy start --foreground` para acompanhar
tokred proxy status               # está rodando? este terminal o usa?
tokred claude                     # Claude Code pelo proxy, só nesta execução
tokred routing status             # decisões registradas até agora (o roteamento é `observe` por padrão)
tokred stats                      # números, incluindo as sugestões
tokred proxy stop
```

O `tokred claude` inicia o proxy sozinho se ele não estiver rodando, repassa todos os argumentos ao `claude` e sai com o código de saída dele. Se o proxy não consegue iniciar, o Claude Code roda como antes.

Para iniciar o proxy no login: `tokred proxy service install` (desfaça com `uninstall`).

### Gerar os arquivos da release na sua máquina

É o que o pipeline de release faz, sem publicar nada:

```sh
dist plan                          # o que uma release conteria (arquivos, checksums, scripts de instalação)
dist build --artifacts=local       # gera os arquivos para o alvo desta máquina
dist build --artifacts=global      # os checksums e os scripts de instalação
```

O `dist` se instala com `cargo install cargo-dist --locked` (ou `brew install cargo-dist`, uma ferramenta de desenvolvimento; quem usa o tokred nunca precisa dele); a versão usada no CI está fixada no `dist-workspace.toml`.

## 2. Publicar uma versão

### Como a entrega funciona

Um push de tag faz tudo. O `.github/workflows/release.yml`, gerado pelo `dist`, compila para macOS, Linux (musl) e Windows em x86_64 e aarch64 um arquivo com o seu `.sha256` e envia tudo para uma GitHub Release, junto com os quatro scripts de instalação copiados de `install/`:

| Asset | O que é |
|---|---|
| `tokred-<target>.tar.gz` / `.zip` | O binário com README, licenças e changelog |
| `tokred-<target>.tar.gz.sha256` / `.zip.sha256` | `<sha256>  <nome do arquivo>`; os scripts de instalação e o `tokred update` o conferem |
| `install.sh`, `install.ps1`, `uninstall.sh`, `uninstall.ps1` | Os scripts que as pessoas executam (`extra-artifacts` no `dist-workspace.toml`) |
| `source.tar.gz`, `sha256.sum` | Adicionados pelo `dist` |

Só isso: **sem tokens, sem segredos, sem repositórios de tap ou bucket, sem registro de pacotes.** O pipeline não precisa de nada além do `GITHUB_TOKEN` do próprio repositório. Quem usa instala assim:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex
```

(`releases/latest/download/` sempre aponta para a versão estável mais nova; `releases/download/vX.Y.Z/`, para uma fixa. Os scripts também existem na branch `main`, em `https://raw.githubusercontent.com/sostenes198/tokred/main/install/`.) Veja em [instalação](installation.pt-BR.md) o que eles fazem.

### A cada versão

```sh
# 1. Parta de uma main verde
git switch main && git pull
cargo fmt --all --check && cargo clippy --workspace --all-targets --all-features -- -D warnings && cargo test --workspace

# 2. Suba a versão: [workspace.package] e o `version = "…"` de cada tokred-* em
#    [workspace.dependencies] precisam ser iguais
sed -i '' 's/"0\.1\.0"/"0.2.0"/' Cargo.toml   # macOS; no Linux: sed -i 's/…/…/'
cargo check --workspace                         # atualiza o Cargo.lock

# 3. CHANGELOG.md e CHANGELOG.pt-BR.md: transforme [Unreleased] em [0.2.0] - AAAA-MM-DD
#    (as notas da GitHub Release saem dele)

# 4. Veja o que seria publicado
dist plan

# 5. Commit, depois tag e push (é aqui que a release começa)
git commit -am "chore: release 0.2.0"
git tag v0.2.0
git push && git push origin v0.2.0
```

Acompanhe a execução na aba **Actions** do repositório (workflow *Release*). Leva de 10 a 20 minutos.

**Ensaio antes.** Envie uma tag de pré-release, como `v0.2.0-rc.1`: o `dist` compila tudo e cria uma *pre-release* no GitHub. Ela não é a "latest", então `releases/latest/download/` não aponta para ela e ninguém a recebe sem querer. Teste com uma versão fixa:

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/download/v0.2.0-rc.1/install.sh | sh -s -- --version 0.2.0-rc.1
```

Confira `tokred --version`, `tokred doctor` e o script de desinstalação; só então envie a tag de verdade (apague a pre-release se quiser removê-la).

### Conferir o resultado

```sh
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/install.sh | sh
tokred --version && tokred doctor
tokred update                     # "up to date", ou ele se substitui
```

No Windows: `irm https://github.com/sostenes198/tokred/releases/latest/download/install.ps1 | iex` e depois `tokred --version`.

### Quando algo dá errado

- **Um job de build falha:** corrija, apague a tag (`git push origin :refs/tags/v0.2.0` e `git tag -d v0.2.0`) e crie de novo. Nada foi publicado se a falha foi antes do job `host`.
- **Uma versão ruim saiu:** publique uma nova versão de correção; a `latest` passa a apontar para ela. Para retirar a ruim, apague a release no GitHub (a tag pode ficar ou sair).
- **Um script de instalação tem um bug:** corrija na `main` e faça uma nova release de correção; a URL raw da `main` serve a correção na hora para quem a usa.

### O que fazer quando o `dist-workspace.toml` muda

Rode `dist generate` e faça commit do `.github/workflows/release.yml` alterado; não edite esse arquivo à mão. O Dependabot sobe versões de actions dentro dele (por exemplo `actions/checkout`); o `allow-dirty = ["ci"]` no `dist-workspace.toml` faz o `dist plan` aceitar isso em vez de falhar com "out of date contents", então esses pull requests podem ser mesclados (mas ele também faz o `dist generate` se recusar a rodar: comente a linha enquanto regenera e depois a restaure). Veja também [instalação](installation.pt-BR.md) e [decisões](decisions.pt-BR.md).
