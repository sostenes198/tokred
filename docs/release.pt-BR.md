[English](release.md) | **Português**

# Executar localmente e publicar uma versão

Duas partes: [rodar o tokred na sua máquina](#1-executar-localmente) e [entregar um binário que as pessoas instalam com Homebrew, Scoop, winget, `cargo install` ou um script](#2-publicar-uma-versão). As verificações de quem contribui (testes, cobertura, lints) estão em [desenvolvimento](development.pt-BR.md).

## 1. Executar localmente

### Compilar e rodar a partir do código-fonte

```sh
cargo build                       # build de debug
cargo run -q -- doctor            # qualquer comando: cargo run -q -- <comando>
cargo build --release             # binário otimizado em target/release/tokred
cargo install --path crates/tokred-cli   # instala como `tokred` em ~/.cargo/bin
```

Requisitos: Rust stable (MSRV 1.89; o `rust-toolchain.toml` instala `rustfmt` e `clippy`).

### Experimentar sem mexer na sua configuração real

Use um sandbox descartável para que nada seja gravado no seu `~/.claude` nem na pasta de dados reais:

```sh
export TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
mkdir "$CLAUDE_CONFIG_DIR/projects"
cargo run -q -- configure --yes   # hooks, agentes e banco vão para o sandbox
cargo run -q -- doctor            # confira o resultado
cargo run -q -- unconfigure --purge
```

`TOKRED_OFFLINE=1` desliga a verificação de versão e a sincronização de preços.

### Rodar o proxy

O [proxy](proxy.pt-BR.md) é opcional. Use o binário de verdade (não o `cargo run`, para que o processo em segundo plano sobreviva ao build):

```sh
tokred proxy start                # em segundo plano; ou `tokred proxy start --foreground` para acompanhar
tokred proxy status               # está rodando? este terminal usa?
tokred claude                     # Claude Code pelo proxy, só nesta execução
tokred routing status             # decisões registradas até agora (o roteamento é `observe` por padrão)
tokred stats                      # números
tokred proxy stop
```

O `tokred claude` sobe o proxy sozinho se ele não estiver rodando, repassa todos os argumentos ao `claude` e sai com o código de saída dele. Se o proxy não subir, o Claude Code roda como antes.

Para subir o proxy no login: `tokred proxy service install` (desfaça com `uninstall`).

### Gerar os arquivos de release na sua máquina

É o que o pipeline de release faz, sem publicar nada:

```sh
dist plan                          # o que uma release conteria (arquivos, instaladores, checksums)
dist build --artifacts=local       # gera os arquivos para o alvo desta máquina
```

O `dist` se instala com `cargo install cargo-dist --locked` (ou `brew install cargo-dist`); a versão usada no CI está fixada no `dist-workspace.toml`.

## 2. Publicar uma versão

### Como a entrega funciona

Um push de tag faz tudo. O `.github/workflows/release.yml`, gerado pelo `dist`, compila para macOS, Linux (musl) e Windows em x86_64 e aarch64 um arquivo com o seu `.sha256`, um instalador shell, um instalador PowerShell e uma fórmula do Homebrew. Depois o `publish-package-managers.yml` atualiza os outros canais:

| Canal | O que é publicado | Onde | Segredo usado |
|---|---|---|---|
| GitHub Release | arquivos, checksums, instaladores | releases de `sostenes198/tokred` | `GITHUB_TOKEN` (automático) |
| Homebrew | fórmula `tokred.rb`, mais o aviso do `tokred configure` | repo `sostenes198/homebrew-tap` | `HOMEBREW_TAP_TOKEN` |
| Scoop | manifesto `bucket/tokred.json` | repo `sostenes198/scoop-bucket` | `SCOOP_BUCKET_TOKEN` |
| winget | pull request para `microsoft/winget-pkgs` | pacote `sostenes198.tokred` | `WINGET_TOKEN` |
| crates.io | todos os crates do workspace (`cargo publish --workspace`) | crates.io | `CARGO_REGISTRY_TOKEN` |
| Script | `tokred-installer.sh` / `tokred-installer.ps1` | anexados à GitHub Release | nenhum |

Quem usa instala assim:

```sh
brew install sostenes198/tap/tokred
scoop bucket add sostenes198 https://github.com/sostenes198/scoop-bucket && scoop install sostenes198/tokred
winget install sostenes198.tokred
cargo install tokred
curl --proto '=https' --tlsv1.2 -LsSf https://github.com/sostenes198/tokred/releases/latest/download/tokred-installer.sh | sh
```

Nenhum deles toca no Claude Code; a pessoa ainda roda `tokred configure`.

### Preparação, uma única vez

O pipeline nunca rodou, e nada disso existe até você criar. Faça uma vez:

1. **Conta do GitHub.** O `gh` desta máquina pode estar logado na conta do trabalho; os repositórios abaixo são da conta pessoal (`sostenes198`). Confira com `gh auth status` e troque com `gh auth switch`.
2. **Tap do Homebrew.** Crie o repo público `sostenes198/homebrew-tap` (pode estar vazio; o `dist` cria o `Formula/tokred.rb`). O prefixo `homebrew-` é o que faz o `brew install sostenes198/tap/tokred` funcionar.
3. **Bucket do Scoop.** Crie o repo público `sostenes198/scoop-bucket`.
4. **Tokens** (GitHub → Settings → Developer settings → Personal access tokens), guardados como segredos do repositório `sostenes198/tokred` (Settings → Secrets and variables → Actions):
   - `HOMEBREW_TAP_TOKEN`: token fine-grained, repositório `homebrew-tap`, permissão *Contents: read and write*.
   - `SCOOP_BUCKET_TOKEN`: o mesmo, para o `scoop-bucket`.
   - `WINGET_TOKEN`: token clássico com o escopo `public_repo`.
   - `CARGO_REGISTRY_TOKEN`: crie em <https://crates.io/settings/tokens> (precisa de conta no crates.io ligada ao GitHub; escopos *publish-new* e *publish-update*).
5. **winget, só na primeira vez.** Envie o primeiro manifesto ao `microsoft/winget-pkgs` à mão (por exemplo com `wingetcreate new <url do zip do Windows>`). Depois que for aceito, o pipeline abre os pull requests de atualização. Deixe o winget para depois se quiser: remova o job `winget` do `publish-package-managers.yml`, senão a execução falha nesse job (os outros canais não são afetados).
6. **Nomes no crates.io.** `tokred`, `tokred-core`, `tokred-proxy` e os demais estão livres hoje. Publicar é permanente: uma versão pode ser retirada (yank), mas nunca apagada nem reutilizada. Rode antes `cargo publish --workspace --dry-run`.

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

**Ensaio antes.** Envie uma tag de pré-release, como `v0.2.0-rc.1`: o `dist` compila tudo e cria uma *pre-release* no GitHub e, por padrão, não roda os jobs de publicação para pré-releases, então Homebrew, Scoop, winget e crates.io ficam intocados. Confirme na execução que os jobs de publicação foram pulados, baixe um arquivo da pre-release e teste. Só então envie a tag de verdade.

### Conferir o resultado

```sh
brew update && brew install sostenes198/tap/tokred
tokred --version && tokred doctor
tokred update                     # mostra o comando certo para o jeito como foi instalado
```

No Windows: `scoop install sostenes198/tokred` ou `winget install sostenes198.tokred`, depois `tokred --version`.

### Quando algo dá errado

- **Um job de build falha:** corrija, apague a tag (`git push origin :refs/tags/v0.2.0` e `git tag -d v0.2.0`) e crie de novo. Nada foi publicado se a falha foi antes dos jobs de publicação.
- **Só um canal falhou** (por exemplo, o winget): rode de novo só o job que falhou na aba Actions; os outros já saíram.
- **Uma versão ruim saiu:** publique uma nova versão de correção. Retire uma versão do crates.io com `cargo yank --version 0.2.0 tokred`; no Homebrew e no Scoop, reverta o commit no tap ou no bucket.
- **Fórmula não encontrada logo após a release:** `brew update`; o job do aviso espera até 10 minutos a fórmula aparecer no tap.

### O que fazer quando o `dist-workspace.toml` muda

Rode `dist generate` e faça commit do `.github/workflows/release.yml` alterado; nunca edite esse arquivo à mão. Veja também [instalação](installation.pt-BR.md) e [decisões](decisions.pt-BR.md).
