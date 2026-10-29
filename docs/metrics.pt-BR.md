[English](metrics.md) | **Português**

# Métricas

O que `tokred stats`, `tokred stats latency` e `tokred metrics` mostram, de onde vêm os números e quais deles são estimativas.

| Comando | Mostra |
|---|---|
| `tokred stats` | Tokens, custo, economia, taxa de acerto do cache e projeção mensal, no terminal |
| `tokred stats latency` | O tempo que os hooks acrescentaram ao Claude Code |
| `tokred metrics` | O mesmo relatório no terminal (30 dias por padrão), mais um arquivo HTML autocontido aberto no navegador |

As flags estão em [comandos](commands.pt-BR.md). `--json` no `stats` imprime `{"period": ..., "stats": {...}}` (com uma chave `proxy` quando o proxy registrou algo).

## Fontes

**Transcripts.** O tokred lê os arquivos de sessão do Claude Code, `<claude>/projects/**/*.jsonl`, de forma incremental para o seu banco: cada execução continua de onde a anterior parou, uma linha que o Claude Code ainda está escrevendo fica para a próxima execução, e uma mensagem que o streaming repete é contada uma vez. De cada mensagem do assistente ele tira o modelo, os quatro contadores de tokens (entrada, saída, escrita no cache, leitura do cache) com a divisão das escritas no cache entre 5 minutos e 1 hora, a sessão, o projeto e se veio de um subagente. Linhas malformadas e mensagens sem uso são ignoradas.

**O proxy.** Quando o [proxy](proxy.pt-BR.md) opcional rodou, ele registrou os próprios dados exatos de cada requisição: o modelo pedido e o modelo que respondeu, tokens, tempo até o primeiro byte e tempo total, a causa provável de uma perda do cache de prompt, a origem (loop principal, subagente, auxiliar, compactação), o tipo de agente de um subagente e o tempo que o próprio proxy acrescentou.

**Mescladas, nunca contadas duas vezes.** Toda requisição que passou pelo proxy também está nos transcripts, então as duas fontes são mescladas antes de qualquer cálculo:

- Uma requisição do proxy é pareada com a mensagem do transcript que tem o mesmo id de mensagem e id de requisição. Uma requisição do proxy sem id (um stream interrompido) é pareada com uma mensagem da mesma sessão cujos quatro contadores são iguais e cujo horário cai entre 5 segundos antes da chegada da requisição e 5 segundos depois do fim da resposta.
- Um par vira uma linha com os números do proxy: o modelo que realmente respondeu, a divisão exata de tokens, a origem e o tipo de agente. O transcript contribui com o projeto.
- As linhas vistas por só um dos lados permanecem: as só do transcript (o proxy estava desligado) e as só do proxy. Uma requisição respondida com status de erro ou sem tokens não é uma mensagem.

Sem o proxy, todo número é calculado só a partir dos transcripts.

## O que o `tokred stats` calcula

Totais, grupos, economia e a taxa do cache cobrem o período dado por `--since` (padrão `7d`). A projeção e a comparação antes/depois também olham fora dele, de propósito.

| Número | Como é calculado | Estimativa? |
|---|---|---|
| Totais | Mensagens, sessões, os quatro contadores de tokens e o custo no período | Não |
| Custo real | Soma, por mensagem, de tokens vezes preço (USD por milhão de tokens). Um modelo sem preço custa 0 e é listado como sem preço | Não |
| Tudo no opus | Os mesmos tokens ao preço mais recente do opus (mensagens de um modelo sem preço ficam de fora). A diferença para o custo real é a **economia do roteamento** | Sim: o que o roteamento mudou é medido, o preço do opus é um contrafactual |
| Economia dos guards | Cada negação de um hook `guard-*` conta `bytes evitados / 4` tokens ao preço de entrada do modelo mais usado pela sessão. Só negações contam: no modo `warn` a leitura ainda acontece | Sim, e rotulada como tal |
| Taxa de acerto do cache de prompt | `leitura do cache / (leitura do cache + escrita no cache + entrada)`, no total e por dia | Não |
| Projeção mensal | Custo médio diário dos últimos 7 dias e dos últimos 30 dias, vezes 30. Dados mais novos que a janela são divididos pelo número real de dias, para uma instalação nova não ser diluída | Sim |
| Antes e depois | Custo por sessão antes e depois do primeiro `tokred configure`; só aparece quando há dados anteriores a ele | Não |

Tudo que é derivado de bytes é rotulado como estimativa. O dinheiro é arredondado em 6 casas decimais, e um período vazio nunca produz NaN.

### Agrupamentos

`--by` reagrupa as linhas:

| `--by` | Uma linha por |
|---|---|
| `model` | Modelo |
| `day` | Dia (UTC) |
| `project` | Pasta do projeto |
| `origin` | Quem enviou a requisição: `main`, `subagent`, `auxiliary`, `compaction` |
| `agent-type` | Tipo de agente de uma requisição de subagente (`Explore`, `Plan`, ...) |

Sem o proxy, `origin` recorre aos transcripts (`main` ou `subagent`) e todo tipo de agente é `(unknown)`.

### A seção Proxy

Quando o proxy registrou algo no período, o `stats` acrescenta uma seção **Proxy**:

| Linha | O que mostra |
|---|---|
| Data source | Mensagens medidas pelo proxy e mensagens conhecidas só pelos transcripts |
| Requests | Requisições que o proxy tratou e quantas o upstream não respondeu com status 2xx |
| Overhead | p50, p95, p99 e máx do tempo que o proxy acrescentou, contra o orçamento de 10 ms; um alerta quando o p95 o ultrapassa |
| Routing | Decisões do período (aplicadas, observadas, mantidas, fallback) com a economia **estimada** das aplicadas, e das observadas caso tivessem sido aplicadas |
| Model routing | O modelo pedido contra o modelo que respondeu, por par: mensagens, tokens e custo |
| Cache misses | Quantas mensagens do loop principal perderam o cache de prompt, entre todas elas, e depois uma tabela por causa provável com os tokens que cada causa escreveu de novo no cache |
| Suggestion | A última sugestão de modelo mais barato ainda não mostrada (veja `tokred suggest`); o `stats` nunca a marca como mostrada |

## Latência acrescentada pelo tokred

Os hooks rodam antes de o Claude Code continuar, então o `tokred stats latency` informa, para cada hook: execuções, p50, p95, p99 e máx em ms; o total e a média acrescentados por prompt; os resultados (`ok`, `error`, `panic`, `timeout_guard`); e um alerta quando o p95 de um hook passa de 50 ms (exatamente 50 está dentro do orçamento). Ele também coloca o tempo total acrescentado ao lado dos tokens que os guards evitaram no mesmo período. Os percentis usam o método do posto mais próximo (nearest-rank).

Um processo não enxerga o próprio tempo de inicialização, então o `tokred doctor --bench` o mede de fora, o salva, e esse valor é somado a cada execução, o que faz o total ser **uma estimativa**. Até você rodar o `doctor --bench` uma vez, o tempo de inicialização conta como 0 e os números são um limite inferior.

## O relatório HTML

O `tokred metrics` grava `<data>/reports/metrics.html` (sempre o mesmo arquivo, sobrescrito) e o abre (`--no-open` só o grava). É um arquivo só, com estilos, scripts e gráficos embutidos, então funciona offline e respeita o seu tema claro ou escuro. Uma seção sem dados diz "no data yet". As seções do proxy (roteamento de modelos, overhead, perdas de cache, custo por origem e tipo de agente) só aparecem quando o proxy registrou algo.

Os botões **7 days / 30 days / All** trocam juntos os cartões de resumo (custo real, economia, tokens evitados, latência) e os gráficos diários; cada visão diz quais dias cobre, e All é o período inteiro do `--since` (30 dias, a menos que você passe outro). A projeção das visões de 7 dias e All vem dos últimos 7 dias, e a da visão de 30 dias, dos últimos 30. As outras seções (modelos, roteamento, guards, latência, audit) sempre cobrem o período inteiro.

## Preços

Os preços vêm da lista pública de preços do [LiteLLM](https://github.com/BerriAI/litellm), mantida no banco do tokred. O `tokred pricing sync` os atualiza; os outros comandos os atualizam no máximo uma vez por dia (desligue com `[pricing] auto_sync = false`, ou defina `TOKRED_OFFLINE=1`). Se a lista não puder ser alcançada, a tabela anterior permanece, e uma tabela vazia é preenchida a partir de um snapshot embutido no binário. As escritas de 1 hora no cache são cobradas pela taxa própria, mais alta.

Um modelo que não está na tabela é cobrado ao preço mais recente da sua família (opus, sonnet, haiku, fable ou mythos) e sinalizado no relatório; um que não pertence a nenhuma família custa 0 e é listado como sem preço.
