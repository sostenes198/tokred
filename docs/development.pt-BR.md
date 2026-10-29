[English](development.md) | **Português**

# Desenvolvimento

Como compilar, executar, testar e experimentar o tokred a partir do código-fonte. Como o código está organizado está em [architecture](architecture.pt-BR.md); como publicar uma versão está em [release](release.pt-BR.md); o fluxo de contribuição está em [CONTRIBUTING](../CONTRIBUTING.pt-BR.md).

## Requisitos

- Rust 1.89 ou mais novo (o MSRV do workspace, verificado no CI). O `rust-toolchain.toml` seleciona o stable com `rustfmt` e `clippy`.
- Para o portão completo local: `cargo install cargo-deny cargo-llvm-cov`.
- Para os testes de shell: `shellcheck`. O smoke test do release também precisa de Python 3.7+ e `curl`. A verificação da documentação precisa de Python 3.

## Compilar e executar

```sh
cargo build
```

```sh
cargo run -q -- doctor
```

`cargo run -q -- <comando>` executa qualquer comando. Um binário compilado do código-fonte funciona de onde estiver, mas o `tokred update` manda reinstalar com o script: só um binário na pasta de instalação se atualiza sozinho. Para rodar o proxy em segundo plano, use o binário compilado e não o `cargo run`, para o proxy sobreviver à compilação.

## O portão de qualidade

O CI executa estes em todo pull request; execute-os antes de dar push.

```sh
cargo fmt --all --check
```

```sh
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

```sh
cargo test --workspace --all-features --no-fail-fast
```

```sh
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
```

```sh
cargo deny check
```

```sh
cargo +1.89 check --workspace --all-targets --locked
```

```sh
python3 scripts/check-docs.py
```

`cargo +1.89 check` é a verificação do MSRV: use a versão escrita em `rust-version`, em `[workspace.package]` do `Cargo.toml`. O `check-docs.py` verifica a documentação: cada documento tem o seu par `.pt-BR.md` com os mesmos títulos e blocos de código, os links resolvem e algumas regras da casa valem (veja o cabeçalho do script).

### Cobertura

Cubra as linhas que você acrescentar. O CI mede a cobertura e publica o `lcov.info`; ele não falha se a cobertura cair, então confira você mesmo.

```sh
cargo llvm-cov --workspace --all-features --summary-only
```

Para listar as linhas que nunca executam:

```sh
cargo llvm-cov --workspace --all-features --lcov --output-path lcov.info
```

No `lcov.info`, `DA:<linha>,0` é uma linha que nunca executou. `cargo llvm-cov report --text` imprime o código-fonte anotado, em que `^0` marca uma região que nunca executou. As visões por linha são a referência: o resumo de regiões e funções do LLVM lê um pouco menos porque cada `?` tem um ramo de erro, e forçar cada falha de I/O acrescentaria ruído, não segurança.

## Testes

Execute tudo, um crate, um arquivo ou um teste:

```sh
cargo test --workspace --no-fail-fast
```

```sh
cargo test -p tokred-core
```

```sh
cargo test -p tokred --test proxy_e2e
```

```sh
cargo test -p tokred a_stopped_proxy
```

| Camada | Onde | O que prova |
|---|---|---|
| Unitário | módulos `#[cfg(test)]` | Lógica pura: classificação, preços, roteamento, compressão, config, funções de merge |
| Integração | `crates/<crate>/tests/` | Arquivos SQLite reais e pastas temporárias; o `tokred-proxy` contra um upstream falso (encaminhamento, streaming, gzip, dashboard); o caminho da rede corporativa (`network.rs`: um proxy HTTP falso respondendo `CONNECT` na frente de um upstream HTTPS falso assinado por uma autoridade descartável, `NO_PROXY`, túneis recusados; `network_logs.rs`: credenciais nunca chegam aos logs) |
| Propriedades | `proptest`, por exemplo `crates/tokred-install/tests/merge_props.rs` | Configurar duas vezes é igual a uma; configurar e desconfigurar restaura o original; hooks alheios sobrevivem; fazer o merge do uso de novo não muda nada |
| Ponta a ponta | `crates/tokred-cli/tests/` | O binário real via `assert_cmd`: `e2e.rs` (configure, hooks, doctor, unconfigure, uninstall em um sandbox), `proxy_e2e.rs` (o processo do proxy: encaminhamento, gravação, checagem de `Host`, `tokred claude`, parada, `proxy start` com saída capturada, retenção, o dashboard ao vivo), `initialize_e2e.rs`, `dashboards.rs` (os dashboards do terminal, o relatório HTML, o cache, `doctor --bench`), `commands.rs` (os comandos que usam a rede, contra um servidor local) |
| Snapshot | `insta`, ao lado dos testes | `settings.json` resultante do merge, `doctor --json`, as seções do terminal, o formato JSON da auditoria |
| Instalador | `tests/installer/install_test.sh`, `install_test.ps1` | Os scripts de instalação e desinstalação contra um release falso servido do disco, em uma home descartável |
| Script de release | `tests/release_scripts/test.sh` | O `scripts/release.sh` em um repositório descartável com um `gh` falso. Não roda no CI: execute-o quando alterar o script |
| Smoke do release | `tests/release/smoke.sh`, `smoke.ps1` | O produto inteiro instalado a partir de um release **publicado**; veja [release](release.pt-BR.md) |

### Regras

- Nenhum teste toca o `~/.claude` real, a pasta de dados real ou a rede. Os testes de ponta a ponta definem `HOME`, `CLAUDE_CONFIG_DIR` e `TOKRED_DATA_DIR` como pastas temporárias (uma com espaço no nome), e o proxy é testado contra servidores falsos locais.
- Toda correção de bug vem com um teste que reproduz o bug.
- Arquivos em `tests/` começam com `#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]`, porque as opções `allow-*-in-tests` do clippy só cobrem código `#[cfg(test)]`.
- As fixtures ficam em `tests/fixtures/` (variações de `settings.json`, incluindo uma com hooks de outra ferramenta de hooks, e cargas de hook em `tests/fixtures/hooks/`). Acrescente uma sempre que encontrar um formato real que o merge precise tratar.

### Snapshots

```sh
INSTA_UPDATE=always cargo test
```

Revise o que mudou antes de fazer commit: `git diff -- '*.snap'`.

### Os scripts de instalação

Os testes do instalador montam um **release falso**: o binário recém-compilado empacotado do jeito que o `dist` empacota (`tokred-<target>/tokred` dentro de `tokred-<target>.tar.gz`, ou um `.zip` com `tokred.exe`) mais o seu `.sha256`, servido por `TOKRED_RELEASE_BASE_URL=file://...`, com `HOME`, `TOKRED_HOME`, pasta de dados e pasta do Claude descartáveis.

```sh
cargo build -p tokred
```

```sh
sh tests/installer/install_test.sh
```

```sh
shellcheck install/*.sh tests/installer/*.sh tests/release/*.sh
```

O `install_test.ps1` faz o mesmo no Windows. Ele altera o `PATH` de usuário e o perfil do PowerShell do runner e os restaura, então fora do CI exige `-AllowRealProfile`. Os scripts são mantidos finos de propósito: todo prompt vive em Rust (`tokred proxy enable`, `tokred uninstall`), onde é testado por unidade pela costura `Io`.

### Testes instáveis e travamentos

- Execute uma suíte suspeita algumas vezes: `for i in 1 2 3; do cargo test -p tokred --no-fail-fast; done`.
- O macOS não tem `timeout`; limite uma execução com `perl -e 'alarm 280; exec @ARGV' cargo test -p tokred`.
- Depois de uma execução interrompida, procure um proxy esquecido com `pgrep -fl "proxy start --foreground"`. Se você também usa o tokred de verdade, esse proxy aparece na lista: pare só os outros, pelo pid, com `kill <pid>`.

## Experimente manualmente

Use um sandbox para que nada seja gravado no seu `~/.claude` nem na sua pasta de dados reais. Execute estes comandos em cada terminal que usar para testar; os exports morrem com o terminal.

```sh
cargo build
```

```sh
unset ANTHROPIC_BASE_URL
```

```sh
export PATH="$PWD/target/debug:$PATH" TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
```

```sh
mkdir "$CLAUDE_CONFIG_DIR/projects"
```

```sh
tokred configure --yes && tokred doctor
```

O `doctor` mostra os hooks e agentes instalados no sandbox. O `unset` impede que um gateway do seu próprio shell entre no sandbox. `TOKRED_OFFLINE=1` desliga a verificação de versão e a sincronização de preços. Ao terminar, pare o proxy, se você subiu um (`tokred proxy stop`), e feche o terminal. Não execute `tokred uninstall` aqui: ele também remove o serviço, a entrada do `PATH` e o wrapper `claude` da sua home de verdade.

### Um hook

Um hook lê JSON no stdin, imprime a sua decisão (se houver) como JSON e sai com 0 aconteça o que acontecer. Ele nunca cria a pasta de dados nem o banco. O hook `guard-read` nega lockfiles, então este comando imprime um `permissionDecision` igual a `deny` e `exit=0`:

```sh
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"Cargo.lock"}}' | tokred hook guard-read; echo "exit=$?"
```

Uma chamada permitida não imprime nada:

```sh
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"README.md"}}' | tokred hook guard-read; echo "exit=$?"
```

`tokred doctor --bench` mede a latência dos hooks; `TOKRED_BENCH_RUNS=3` o torna rápido. Antes de implementar um hook, leia a [documentação oficial de hooks](https://code.claude.com/docs/en/hooks) para o formato exato de entrada e saída, e registre qualquer divergência em [decisions](decisions.pt-BR.md).

### O proxy, sem chave de API

Aponte o proxy para um upstream falso que devolve uma mensagem com contagem de tokens. No terminal A, suba a API falsa na porta 9000:

```sh
python3 -I - <<'PY'
import itertools, json, http.server
ids = itertools.count(1)
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        body = json.dumps({"id": f"msg_{next(ids)}", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
                           "content": [{"type": "text", "text": "hi"}], "stop_reason": "end_turn",
                           "usage": {"input_tokens": 120, "output_tokens": 15}}).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
http.server.HTTPServer(("127.0.0.1", 9000), H).serve_forever()
PY
```

No terminal B (com os exports do sandbox), aponte o proxy para ela e suba-o em uma porta só sua, para que um tokred que você use de verdade na porta padrão não entre na história:

```sh
perl -pi -e 's#^upstream = .*#upstream = "http://127.0.0.1:9000"#' "$TOKRED_DATA_DIR/tokred.toml"
```

```sh
tokred proxy enable --port 1050 --yes && tokred proxy status
```

Envie uma requisição pequena (sem ferramentas, uma mensagem: o tokred a vê como `auxiliary`) e depois uma que leva um id de agente (um `subagent`):

```sh
curl -s http://127.0.0.1:1050/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' -H 'content-type: application/json' -d '{"model":"claude-sonnet-4-5","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}'
```

```sh
curl -s http://127.0.0.1:1050/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' -H 'x-claude-code-agent-id: a1' -H 'content-type: application/json' -d '{"model":"claude-sonnet-4-5","max_tokens":16,"tools":[{"name":"Grep","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"find where main is defined"}]}'
```

Confira:

- as respostas são exatamente o que o upstream falso devolveu (em `observe` o proxy não altera nada);
- `tokred routing status` lista duas decisões, ambas `kept` (nada é alterado em `observe`);
- `sqlite3 "$TOKRED_DATA_DIR/tokred.db" "select origin, model_requested, model_effective, input, output from proxy_requests"` mostra duas linhas (`auxiliary` e `subagent`) com 120 tokens de entrada e 15 de saída;
- `tokred stats` tem uma seção **Proxy** contando duas requisições, e `tokred stats --by origin` lista as duas origens;
- `tokred dashboard --no-open` imprime um endereço de uso único para o [dashboard](dashboard.pt-BR.md); abra-o no navegador para ver a sessão;
- um `Host` errado é recusado: `curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: evil.example' http://127.0.0.1:1050/v1/messages -d '{}'` imprime `421`.

Depois pare tudo: `tokred proxy stop` deve dizer que o proxy parou, `tokred proxy status` deve dizer que ele não está rodando, e pare a API falsa com Ctrl-C no terminal A.

### O proxy com o Claude Code de verdade

Você precisa do seu próprio login (uma chave de API, ou o `claude` logado em uma assinatura). Um `CLAUDE_CONFIG_DIR` descartável significa logar de novo dentro dele; isso é intencional.

```sh
tokred proxy enable --port 1050 --yes
```

```sh
tokred claude
```

Trabalhe por um minuto com pelo menos um subagente (por exemplo, peça para ele "explorar o repositório") e depois veja `tokred routing status`, `tokred suggest` e `tokred proxy status`. Os números de `input`, `output` e cache de uma requisição em `proxy_requests` devem bater com o `usage` da mesma mensagem em `$CLAUDE_CONFIG_DIR/projects/**/*.jsonl`.

O `tokred compression on` registra o servidor MCP `tokred_retrieve` na configuração de usuário do Claude Code (`~/.claude.json`, com um backup ao lado). Só `tokred compression on`, `tokred compression off` e `tokred uninstall` editam esse arquivo; quando a pasta do sandbox não tem `.claude.json`, o tokred recorre ao da sua home, então faça também `export HOME=$(mktemp -d)` antes de experimentá-los.

## Integração contínua

| Workflow e job | O que executa |
|---|---|
| CI `fmt`, `clippy`, `docs`, `deny`, `msrv`, `documentation` | As verificações do portão de qualidade, no Linux (`docs` é o `cargo doc`, `documentation` é o `scripts/check-docs.py`) |
| CI `test` | `cargo test` no Linux, macOS e Windows |
| CI `coverage` | `cargo llvm-cov`, publicando o `lcov.info` |
| CI `shellcheck` | `install/*.sh`, `tests/installer/*.sh`, `tests/release/*.sh` |
| CI `installer`, `installer-windows` | Os testes do instalador no Ubuntu, macOS e Windows |
| `Release` | Compila os arquivos em uma tag; veja [release](release.pt-BR.md) |
| `Release smoke` | Instala o release publicado em sistemas reais; veja [release](release.pt-BR.md) |
| `Cut a release` | Executa o `scripts/release.sh` manualmente pela aba Actions; veja [release](release.pt-BR.md) |

Um teste que só passa no seu sistema operacional não está pronto.

## Convenções que já pegaram gente

- **Helpers só de `cfg(unix)`.** Um helper usado só por um teste `#[cfg(unix)]` também precisa de `#[cfg(unix)]`, senão vira código morto e o `-D warnings` quebra o build do Windows.
- **Processos desacoplados no Windows.** Um filho herda os handles herdáveis do pai, então um `tokred proxy start` com saída capturada esperaria o proxy. O proxy é iniciado por `Start-Process`, que não os herda (veja [decisions](decisions.pt-BR.md)). Testes que executam `proxy start` com saída capturada impõem um prazo em uma thread em vez de confiar no timeout do próprio comando.
- **O macOS não tem `timeout`:** use a forma com `perl` acima.
- **Fins de linha.** O `.gitattributes` mantém os arquivos de texto em LF e os scripts de PowerShell em CRLF em todo checkout.
- **Snapshot de preços.** O `scripts/update-price-snapshot.sh` regenera a lista de preços embutida.
