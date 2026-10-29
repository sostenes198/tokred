[English](testing.md) | **Português**

# Testar o tokred localmente

Duas formas de testar: [rodar os testes automatizados](#1-testes-automatizados) e [experimentar a ferramenta à mão](#2-experimentar-à-mão), sem mexer na sua configuração real do Claude Code. Como os testes são organizados e o que eles precisam seguir está em [desenvolvimento](development.pt-BR.md); como publicar uma versão está em [release](release.pt-BR.md).

## 1. Testes automatizados

Requisitos: Rust stable (o `rust-toolchain.toml` instala `rustfmt` e `clippy`), mais `cargo install cargo-deny cargo-llvm-cov` para o gate completo. Não precisa de rede nem de um Claude Code de verdade: todo teste usa pastas temporárias e servidores falsos.

```sh
cargo test --workspace --no-fail-fast        # tudo
cargo test -p tokred-core                    # um crate
cargo test -p tokred --test proxy_e2e        # um arquivo de teste (o binário real, proxy de ponta a ponta)
cargo test -p tokred a_stopped_proxy         # testes cujo nome contém este texto
cargo test -p tokred -- --nocapture          # mostra o que os testes imprimem
```

| Onde | O que prova |
|---|---|
| módulos `#[cfg(test)]` | Lógica pura: classificação, preços, decisões de roteamento, compressão, configuração |
| `crates/<crate>/tests/` | Arquivos SQLite reais, pastas temporárias reais, o proxy contra um upstream falso (repasse, streaming, gzip) |
| `crates/tokred-cli/tests/e2e.rs` | O binário `tokred` real: `configure`, hooks, `doctor`, `unconfigure` num sandbox |
| `crates/tokred-proxy/tests/network.rs` | O caminho da rede corporativa: um proxy HTTP falso que responde `CONNECT` na frente de um upstream HTTPS falso assinado por uma autoridade descartável (`rcgen`), `NO_PROXY`, túneis recusados, valores inválidos; o `network_logs.rs` confere que credenciais nunca chegam aos logs |
| `crates/tokred-cli/tests/proxy_e2e.rs` | O binário real rodando o proxy: repasse, registro, checagem do `Host`, `tokred claude`, comandos de conselho, o processo terminando após `proxy stop`, `proxy start` com a saída capturada (no Windows um proxy destacado fazia quem o chamava esperar; a primeira execução do CI no Windows é a verificação da partida por `Start-Process`) |

### O gate completo (o que o CI roda)

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --no-fail-fast
RUSTDOCFLAGS="-D warnings" cargo doc --workspace --no-deps
cargo deny check
cargo +1.89 check --workspace --all-targets --locked
```

### Cobertura

A meta é 100% das linhas.

```sh
cargo llvm-cov --workspace --no-fail-fast --summary-only
cargo llvm-cov --workspace --no-fail-fast --lcov --output-path lcov.info   # `DA:<linha>,0` = nunca executou
```

### Testes instáveis e travamentos

- Rode várias vezes uma suíte suspeita: `for i in 1 2 3; do cargo test -p tokred --no-fail-fast; done`.
- O macOS não tem `timeout`; limite uma execução com `perl -e 'alarm 280; exec @ARGV' cargo test -p tokred`.
- Depois de uma execução interrompida, procure um proxy que ficou para trás: `pgrep -fl "proxy start --foreground"` e encerre com `pkill -f "proxy start --foreground"`.
- Snapshots (`insta`): `INSTA_UPDATE=always cargo test` e depois revise com `git diff -- '*.snap'`.

O CI roda a suíte em Linux, macOS e Windows; um teste que passa só no seu sistema não está pronto.

## 2. Experimentar à mão

Use um sandbox para que nada seja gravado no seu `~/.claude` nem na pasta de dados reais. Rode isto em cada terminal novo que usar para testar:

```sh
cargo build
export PATH="$PWD/target/debug:$PATH"
export TOKRED_DATA_DIR=$(mktemp -d) CLAUDE_CONFIG_DIR=$(mktemp -d) TOKRED_OFFLINE=1
mkdir "$CLAUDE_CONFIG_DIR/projects"
tokred configure --yes && tokred doctor
```

O `doctor` deve mostrar os hooks e os agentes instalados no sandbox. Ao terminar: `tokred unconfigure --purge` e feche o terminal (os `export` morrem com ele).

### Um hook

```sh
echo '{"session_id":"s1","hook_event_name":"PreToolUse","tool_name":"Read","tool_input":{"file_path":"Cargo.lock"}}' \
  | tokred hook guard-read; echo "exit=$?"       # uma chamada permitida não imprime nada e sai com 0
tokred doctor --bench                            # latência dos hooks; TOKRED_BENCH_RUNS=3 para uma rodada rápida
```

### O proxy, sem chave de API

Aponte o proxy para um upstream falso que devolve uma mensagem com contagem de tokens.

```sh
# Terminal A: a API falsa na porta 9000
python3 -I - <<'PY'
import json, http.server
class H(http.server.BaseHTTPRequestHandler):
    def do_POST(self):
        self.rfile.read(int(self.headers.get("content-length", 0)))
        body = json.dumps({"id": "msg_1", "type": "message", "role": "assistant", "model": "claude-sonnet-4-5",
                           "content": [{"type": "text", "text": "hi"}], "stop_reason": "end_turn",
                           "usage": {"input_tokens": 120, "output_tokens": 15}}).encode()
        self.send_response(200); self.send_header("content-type", "application/json")
        self.send_header("content-length", str(len(body))); self.end_headers(); self.wfile.write(body)
http.server.HTTPServer(("127.0.0.1", 9000), H).serve_forever()
PY
```

```sh
# Terminal B (com os export do sandbox acima)
sed -i '' 's#^upstream = .*#upstream = "http://127.0.0.1:9000"#' "$TOKRED_DATA_DIR/tokred.toml"   # Linux: sed -i sem ''
tokred proxy start && tokred proxy status

# uma requisição pequena (sem ferramentas, uma mensagem: o tokred a vê como "auxiliar") e depois uma de "subagente" (leva um agent id)
curl -s http://127.0.0.1:8787/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' \
  -H 'content-type: application/json' -d '{"model":"claude-sonnet-4-5","max_tokens":16,"messages":[{"role":"user","content":"hi"}]}'
curl -s http://127.0.0.1:8787/v1/messages -H 'x-api-key: test' -H 'x-claude-code-session-id: s1' \
  -H 'x-claude-code-agent-id: a1' -H 'content-type: application/json' \
  -d '{"model":"claude-sonnet-4-5","max_tokens":16,"tools":[{"name":"Grep","input_schema":{"type":"object"}}],"messages":[{"role":"user","content":"find where main is defined"}]}'

tokred routing status        # as duas requisições foram classificadas; o modo é observe, então os modelos foram mantidos
tokred proxy stop
```

O que conferir:

- as respostas que você recebe são exatamente o que o servidor falso devolveu (o proxy não altera nada em `observe`);
- `sqlite3 "$TOKRED_DATA_DIR/tokred.db" "select origin, model_requested, model_effective, input, output from proxy_requests"` mostra duas linhas (`auxiliary` e `subagent`) com 120 tokens de entrada e 15 de saída;
- uma requisição com `Host: evil.example` é recusada: `curl -s -o /dev/null -w '%{http_code}\n' -H 'Host: evil.example' http://127.0.0.1:8787/v1/messages -d '{}'` imprime `421`;
- parar de verdade encerra o processo: `pgrep -fl "proxy start --foreground"` não imprime nada.

### O proxy com o Claude Code de verdade

Você precisa do seu próprio login (uma chave de API, ou o `claude` logado numa assinatura). Usar um `CLAUDE_CONFIG_DIR` descartável significa logar de novo dentro dele; é de propósito.

```sh
tokred claude                # igual ao `claude`, pelo proxy, só nesta execução
# trabalhe por um minuto, com pelo menos um subagente (por exemplo, peça para "explorar o repositório")
tokred routing status        # decisões das requisições dos subagentes
tokred suggest               # conselhos, depois de vários prompts simples
tokred proxy status
```

Para comparar com o que o próprio Claude Code registrou: os números de `input`, `output` e cache de uma requisição em `proxy_requests` precisam bater com o `usage` da mesma mensagem em `$CLAUDE_CONFIG_DIR/projects/**/*.jsonl`.

### Chaves de roteamento e compressão

```sh
tokred routing on            # [routing] mode = "enforce"; reinicie o proxy para aplicar
tokred routing off
tokred compression on        # registra o servidor MCP tokred_retrieve em ~/.claude.json (backup .tokred.bak)
tokred compression status
tokred compression off
```

O `compression on` é o único comando que edita a configuração de usuário do Claude Code. Quando a pasta do sandbox não tem um `.claude.json`, o tokred usa o `~/.claude.json`; por isso aponte o `HOME` para uma pasta temporária também (`export HOME=$(mktemp -d)`) antes de experimentar.

### Status line

```sh
echo '{"session_id":"s1"}' | tokred statusline     # a sugestão pendente, ou os números da sessão
```

### Limpeza

```sh
tokred proxy stop; tokred unconfigure --purge
# e pare a API falsa com Ctrl-C no terminal A
```
