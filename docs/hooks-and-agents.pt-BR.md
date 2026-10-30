[English](hooks-and-agents.md) | **Português**

# Hooks e agentes

Como o tokred se conecta ao Claude Code sem o proxy: sete hooks que rodam em volta dos seus prompts e chamadas de ferramentas, e seis subagentes com um modelo fixo cada. O `tokred configure` instala os dois; o `tokred unconfigure` os remove.

## Os sete hooks

Cada hook é registrado no `settings.json` como `"<caminho>/tokred" hook <nome>` com timeout de 5 segundos. Os formatos de entrada e saída seguem a [referência de hooks](https://code.claude.com/docs/en/hooks) do Claude Code.

| Hook | Evento do Claude Code | Matcher | O que decide | Modo padrão |
|---|---|---|---|---|
| `route` | `UserPromptSubmit` | nenhum | Classifica o prompt e sugere o subagente certo (uma linha, só quando a classe mudou) | `warn` |
| `enforce` | `PreToolUse` | `Agent\|Task` | Compara o subagente escolhido pelo Claude com a última classificação | `warn` |
| `guard-read` | `PreToolUse` | `Read` | Bloqueia lockfiles, arquivos gerados, arquivos enormes lidos inteiros e releituras sem mudança | `enforce` |
| `guard-search` | `PreToolUse` | `Grep\|Glob` | Nega buscas amplas demais em repositórios grandes | `warn` |
| `session-check` | `Stop` | nenhum | Avisa você (nunca o modelo) quando o contexto passa de um limite | `warn` |
| `session-start` | `SessionStart` | nenhum | Registra a sessão; depois de `/clear` ou `/compact` volta a permitir releituras | sem modo |
| `model-guard` | `PreModelSwitch` | nenhum | Avisa o que uma troca de modelo custa em cache de prompt, ou pede que você confirme | `warn` |

### Modos

Todo hook, exceto o `session-start`, respeita o seu modo em `[modes]` do `tokred.toml`:

| Modo | Comportamento |
|---|---|
| `off` | Não faz nada e não registra nada |
| `warn` | Aconselha; a chamada segue |
| `enforce` | Nega a chamada (`on` é aceito como outro nome) |

Para desligar um hook, defina o modo dele como `"off"`; não precisa mexer no `settings.json`:

```toml
[modes]
guard_read = "off"
```

As chaves são `route`, `enforce`, `guard_read`, `guard_search`, `session_check` e `model_guard`. `route` e `session_check` nunca negam, então `enforce` se comporta como `warn` para eles. `model_guard` também nunca nega: com `"on"` ele pede que você confirme a troca. Os limites de cada hook (`max_read_bytes`, `deny_globs`, `large_repo_files`, `context_warn_tokens`, `min_context_tokens`) estão em [configuração](configuration.pt-BR.md).

Como uma resposta chega ao Claude Code:

| Resposta | Saída |
|---|---|
| Negar (`enforce`) | `permissionDecision: "deny"` com um motivo curto |
| Aconselhar (`warn`) | `additionalContext`: uma linha, sem decisão |
| Permitir | Sem saída |
| Perguntar (`model-guard` com `on`) | `permissionDecision: "ask"`: o Claude Code pede que você confirme a troca |
| Mensagem para você (`session-check`, `model-guard`) | `systemMessage`; nunca chega ao modelo |

O tokred nunca envia `permissionDecision: "allow"`, porque o Claude Code então pularia o seu prompt de permissão (ou, numa troca de modelo, a confirmação que ele mostra enquanto o cache de prompt está quente).

### O que cada hook decide

**`guard-read`**

1. **Lockfiles e arquivos gerados.** Um caminho que casa com `deny_globs` (relativo à pasta da sessão) é negado. Padrões: `package-lock.json`, `pnpm-lock.yaml`, `yarn.lock`, `Cargo.lock`, `poetry.lock`, `*.min.js`, `*.map` e as pastas `dist/`, `build/`, `node_modules/`, `target/`, `.git/`.
2. **Arquivo grande.** Acima de `max_read_bytes`, sem `offset` nem `limit`: negado, pedindo uma leitura por faixa. Imagens e PDFs são isentos.
3. **Releitura.** Uma leitura do mesmo caminho, `offset` e `limit` cujo conteúdo não mudou desde uma leitura anterior da mesma sessão é negada: o conteúdo já está no contexto. Depois de `/clear` ou `/compact`, só contam as leituras feitas após esse ponto.

Os subagentes são acompanhados separadamente, porque o contexto de um subagente não contém o que o pai leu. Os bytes evitados são registrados; no modo `warn` nada é evitado, então a economia é 0.

**`guard-search`**

Uma busca que começa na raiz do repositório (a pasta mais próxima que tem um `.git`, ou a pasta da sessão) é ampla quando é um `Grep` sem `glob` nem `type`, ou um `Glob` cujo padrão casa com todos os arquivos (`*`, `**`, `**/*`, ...). Só as buscas amplas são verificadas: os arquivos do repositório são contados (a contagem fica em cache e é abandonada após 100 ms, o que permite a busca). Acima de `large_repo_files` a busca é negada (`enforce`) ou desaconselhada (`warn`).

**`route`**

Um classificador heurístico (sem chamar modelo) sobre palavras-chave em português e inglês, o tamanho do prompt e menções a arquivos. Ele coloca o prompt em `architecture`, `planning`, `implementation`, `review`, `search`, `summary` ou `other`. Prompts muito curtos que não mencionam arquivo nunca recebem sugestão. Quando a classe mudou desde o último prompt acionável da sessão, ele acrescenta uma linha:

```text
tokred: implementation task -> delegate to the tokred-dev subagent if it involves more than one edit.
```

Com o [proxy](proxy.pt-BR.md) rodando e `[suggest] channel = "message"`, o `route` também pode mostrar uma mensagem de uma linha dizendo que um modelo mais barato para o loop principal teria bastado, porque os prompts recentes eram simples. Ela aparece uma vez, só em um momento seguro (o cache de prompt já está frio, ou nada rodou desde o último `/compact`), e nunca muda o modelo.

**`enforce`**

Compara o subagente escolhido pelo Claude com a última classificação da sessão. Agentes que não são do tokred são sempre permitidos, e uma sessão sem nada classificado também. Em caso de divergência, `warn` acrescenta um aviso curto e `enforce` nega, indicando o agente certo. Só a conversa principal é verificada.

**`session-start`**

Registra a sessão e a pasta dela, e marca o corte depois de `/clear` ou `/compact` para o `guard-read`. Não imprime nada. É também uma rede de segurança do proxy: quando `[proxy] enabled = true`, a sessão está apontada para o proxy e o proxy não responde rápido, ele inicia o proxy em segundo plano sem esperar, de modo que um proxy que morreu não deixa o Claude Code trancado do lado de fora.

**`session-check`**

Lê o uso de tokens da última mensagem da conversa principal no transcript e estima o tamanho do contexto. Acima de `context_warn_tokens` ele diz a você para considerar `/compact` ou `/clear`. Nunca bloqueia a parada e avisa uma vez por sessão: de novo só depois que o contexto cresceu metade do limite, ou depois de um `/clear` ou `/compact`. O número pode ficar uma mensagem atrasado.

**`model-guard`**

Roda antes de o Claude Code aplicar uma troca de modelo que você pediu: `/model`, o seletor de modelo, a opção de modelo no `/config` ou o fast mode. Exige o Claude Code 2.1.251 ou mais novo, a versão que adicionou o evento `PreModelSwitch`, e não roda para trocas que o próprio Claude Code faz, como um fallback. O Claude Code envia o tamanho do contexto, se o cache de prompt ainda está quente e o preço estimado de gravar o contexto no cache do novo modelo, então o tokred não estima nada. Com o cache quente e o contexto com pelo menos `min_context_tokens` tokens (100.000 por padrão), `warn` mostra uma mensagem de uma linha e `on` pede que você confirme a troca, ambos com esses números. Qualquer outra troca (contexto pequeno, cache frio, modelo que já está em uso) fica sem resposta, e vale a confirmação do próprio Claude Code. O `ask` só funciona no `/model` interativo: nas outras superfícies (`-p`, `/config`, Agent SDK) o Claude Code o trata como recusa. Uma troca de modelo nunca recebe `deny`.

### Garantias

- **Fail-open.** Diante de qualquer erro, pânico, entrada inválida ou timeout, um hook sai com `0` sem saída e registra o problema em `<data>/logs/`. Um limite de tempo de 2 segundos encerra um hook que travar. Se a pasta de dados ou o banco não existir, o hook não faz nada e nunca os cria. Um `tokred.toml` ilegível significa os padrões. Um nome de hook desconhecido também sai com `0`, então o Claude Code nunca vê o código de saída `2`, que significa "bloquear".
- **Local e rápido.** Um hook nunca usa a rede. O `tokred doctor --bench` mede quanto cada um demora, e o `tokred stats latency` sinaliza um hook cujo p95 passa de 50 ms.
- **Medido.** Uma execução que decide algo, e toda execução que falha, é registrada com sua duração e resultado (`ok`, `error`, `panic`, `timeout_guard`), depois que a resposta foi escrita. Um hook que não faz nada (o modo dele é `off`, ou a chamada não é uma que ele verifica) não registra nada. O `tokred stats latency` mostra isso.
- **Contexto mínimo.** Todo `additionalContext` tem uma linha e só é emitido quando algo muda.

## Os seis subagentes

O `tokred configure` grava seis arquivos de agente em `<claude>/agents/`, cada um com um modelo fixo:

| Arquivo | Modelo | Ferramentas | Uso |
|---|---|---|---|
| `tokred-architect.md` | opus | Read, Grep, Glob | Arquitetura, decisões de design, trade-offs |
| `tokred-planner.md` | opus | Read, Grep, Glob | Planejamento e quebra de tarefas grandes |
| `tokred-dev.md` | sonnet | Read, Edit, Write, Grep, Glob, Bash | Implementação, refatoração, correção de bugs |
| `tokred-reviewer.md` | sonnet | Read, Grep, Glob, Bash | Revisão de código e de testes |
| `tokred-explorer.md` | haiku | Read, Grep, Glob | Busca no código (somente leitura) |
| `tokred-summarizer.md` | haiku | Read, Grep, Glob | Resumos de docs, logs e saídas longas (somente leitura) |

Cada agente é instruído a não repetir a tarefa nem descrever o processo, a preferir edições pontuais a reescrever arquivos, e a responder apenas neste formato curto, que mantém pequeno o resultado devolvido à conversa principal:

```text
RESULT: <1-3 lines>
FILES: <paths changed or relevant>
PENDING: <list or "none">
```

### Instalar, atualizar e editar agentes

- Os arquivos têm o prefixo `tokred-` e uma linha `<!-- managed-by: tokred vX.Y.Z -->`. Arquivos sem o prefixo nunca são tocados.
- O tokred lembra o hash de cada arquivo que gravou. Em uma atualização, um arquivo que ainda confere é sobrescrito com a nova versão. Um arquivo que você editou é mantido, a nova versão é gravada ao lado como `tokred-x.md.new`, e o `tokred doctor` avisa. O `tokred configure --force` sobrescreve as suas edições.

## O que configure e unconfigure fazem no `settings.json`

O `tokred configure` adiciona cada hook do tokred como um grupo próprio, com o seu matcher, e não altera mais nada no arquivo:

```json
{ "matcher": "Read",
  "hooks": [{ "type": "command", "command": "\"/path/to/tokred\" hook guard-read", "timeout": 5 }] }
```

O tokred reconhece as suas entradas pelo comando (um executável cujo nome de arquivo começa com `tokred`, como `tokred`, `tokred.exe` ou `tokred-0.4.0`, seguido de `hook <nome>`). Só essas entradas são adicionadas, substituídas ou removidas; os seus hooks e qualquer coisa que o tokred não entenda permanecem como estão. Antes de alterar o arquivo, o `configure` salva o anterior como `settings.json.tokred.bak` (só o último backup é mantido). Se o `settings.json` não for um JSON válido, o `configure` para e não escreve nada. Rodá-lo de novo é seguro.

O `tokred unconfigure` remove as entradas de hook do tokred e os arquivos de agente que o tokred instalou. Ele também grava `settings.json.tokred.bak`, apaga arquivos `.new` pendentes e mantém um agente que você editou ou cuja origem ele não consegue verificar, informando cada um. Deixa o proxy e o `ANTHROPIC_BASE_URL` dele em paz, e uma atualização nunca recoloca os hooks: só o `tokred configure` faz isso. Os dois comandos estão descritos em [comandos](commands.pt-BR.md).

O tokred mantém os seus hooks em grupos próprios e nunca altera hooks que pertencem a outras ferramentas ou a você, então pode ser usado junto com ferramentas como o rtk; o `tokred doctor` aponta outras ferramentas de hook que encontrar.
