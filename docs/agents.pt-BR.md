[English](agents.md) | **Português**

# Subagentes

O `tokred configure` instala seis subagentes em `<claude>/agents/`, cada um com modelo fixo:

| Arquivo | Modelo | Ferramentas | Uso |
|---|---|---|---|
| `tokred-architect.md` | opus | Read, Grep, Glob | Arquitetura, decisões de design, trade-offs |
| `tokred-planner.md` | opus | Read, Grep, Glob | Planejamento e quebra de tarefas grandes |
| `tokred-dev.md` | sonnet | Read, Edit, Write, Grep, Glob, Bash | Implementação, refatoração, correção de bugs |
| `tokred-reviewer.md` | sonnet | Read, Grep, Glob, Bash | Revisão de código e testes |
| `tokred-explorer.md` | haiku | Read, Grep, Glob | Busca no código (somente leitura) |
| `tokred-summarizer.md` | haiku | Read, Grep, Glob | Resumo de docs, logs e saídas longas (somente leitura) |

Os templates ficam em `templates/agents/` e são escritos em inglês. Frontmatter: `name`, `description` (quando delegar), `model`, `tools`.

## Regras comuns e formato de retorno

Todo agente é instruído a não recapitular a tarefa nem descrever o processo, a preferir edições pontuais a reescrever arquivos e a responder **apenas** neste formato curto, que reduz tokens e mantém pequeno o resultado devolvido ao contexto principal:

```
RESULT: <1-3 lines>
FILES: <paths changed or relevant>
PENDING: <list or "none">
```

(Os rótulos ficam em inglês porque os templates são em inglês.)

## Arquivos gerenciados

- Os arquivos têm o prefixo `tokred-`. A primeira linha do corpo, depois do frontmatter, é `<!-- managed-by: tokred vX.Y.Z -->`.
- A tabela `installed_files` guarda o path, o hash blake3 e a versão de cada arquivo.
- No update: se o arquivo em disco **for igual** ao hash registrado, é sobrescrito; se **for diferente** (você editou), é deixado como está e a versão nova vai para `tokred-x.md.new` ao lado, e o `tokred doctor` avisa. `--force` sobrescreve mesmo assim.
- Arquivos sem o prefixo **nunca** são tocados. O `tokred unconfigure` mantém os agentes que você editou.
