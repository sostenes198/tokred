[English](README.md) | **Português**

# Spike do proxy (fase F7)

O `proxy_probe.py` é um proxy de repasse **descartável**, usado para responder às perguntas em aberto da spec antes de construir o `tokred-proxy` de verdade. Não faz parte do tokred e não é distribuído. Nunca registra valores de credenciais nem conteúdo de mensagens: só nomes de cabeçalhos, valores de `x-claude-code-*`, `anthropic-beta`/`anthropic-version`, fatos estruturais do corpo (modelo, stream, contagens) e o uso de cada resposta, em `/tmp/tokred-spike/log.jsonl`.

```sh
mkdir -p /tmp/tokred-spike
python3 -I scripts/spike/proxy_probe.py &      # imprime a porta
ANTHROPIC_BASE_URL=http://127.0.0.1:<porta> claude -p "Reply with exactly one word: ok" --model haiku < /dev/null
```

Sonda: reescreve o modelo só das requisições de **subagentes** (o loop principal nunca é tocado):

```sh
SPIKE_REWRITE="sonnet:claude-haiku-5-5" SPIKE_AGENT_ONLY=1 python3 -I scripts/spike/proxy_probe.py &
```

## Testando uma assinatura claude.ai sem mexer no seu login

Use uma pasta de config descartável do Claude Code para não alterar seu login real:

```sh
export CLAUDE_CONFIG_DIR=$(mktemp -d)
env -u ANTHROPIC_API_KEY claude auth login --claudeai      # abre o navegador
env -u ANTHROPIC_API_KEY claude auth status                # authMethod NÃO pode ser "api_key"
python3 -I scripts/spike/proxy_probe.py &                  # anote a porta
env -u ANTHROPIC_API_KEY ANTHROPIC_BASE_URL=http://127.0.0.1:<porta> \
  claude -p "Reply with exactly one word: ok" --model haiku < /dev/null
cat /tmp/tokred-spike/log.jsonl                            # status 200? quais cabeçalhos? quais betas?
```

Depois repita com `SPIKE_REWRITE`/`SPIKE_AGENT_ONLY` e o prompt do subagente Explore para ver se o tráfego de assinatura tolera um modelo reescrito. Registre o resultado em `docs/decisions.md`. Apague a pasta descartável no fim.
