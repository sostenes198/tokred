[English](metrics.md) | **Português**

# Métricas

Os números por trás de `tokred stats`, `tokred stats latency` e do relatório HTML vêm do crate `tokred-metrics`: funções puras sobre dados, mais um adaptador de sistema de arquivos. Esta página descreve o que elas realmente fazem, para que os números signifiquem o que dizem. Veja o [desenho original](archive/spec-v1.pt-BR.md) (seção 10) e a [spec atual](spec.pt-BR.md) para ver como o proxy o estende.

## Ingestão

O `tokred-metrics` lê `<claude>/projects/**/*.jsonl` de forma incremental para a tabela `usage`.

- De cada linha com `type: "assistant"` ele extrai `sessionId`, `timestamp` (RFC 3339, truncado para segundos), `requestId`, `isSidechain` e `message.{id, model, usage}`, onde `usage` traz `input_tokens`, `output_tokens`, `cache_creation_input_tokens` e `cache_read_input_tokens`. Todos os outros tipos de linha (`user`, `attachment`, `queue-operation`, ...) são ignorados.
- Mensagens com modelo `<synthetic>` e linhas sem `usage` são puladas.
- O streaming pode repetir uma mensagem, então as linhas são deduplicadas por `message_key = "<message.id>:<requestId>"`; vale a primeira linha vista.
- `project` é o nome do diretório do projeto. Uma linha é marcada `is_subagent` quando vem de um arquivo em `<project>/<session>/subagents/` ou tem `isSidechain: true`. Linhas de subagente sem `sessionId` usam o nome do diretório da sessão.
- Linhas JSON malformadas são puladas, nunca são fatais.

**Leitura incremental.** O `ingest_state` guarda, por arquivo, o offset de bytes já consumido e o mtime. Cada execução retoma do offset e só consome linhas completas: uma última linha sem `\n` (o Claude Code ainda está escrevendo) fica para a próxima execução, então o offset salvo sempre fica logo após uma quebra de linha. Um arquivo menor que o offset salvo foi truncado ou rotacionado e é lido de novo desde o início; por causa da deduplicação, reler nunca conta em dobro. O offset só é salvo depois que as linhas foram gravadas. Um arquivo substituído por outro maior não é detectado (não há hash de conteúdo); a leitura retoma no offset antigo, então a primeira linha parcial é pulada como malformada e os bytes antes do offset não são relidos.

Tudo isso passa pela porta `TranscriptFs` (listar arquivos com tamanho e mtime, ler bytes a partir de um offset) e pelas portas `UsageRepo`/`IngestRepo`, então todo o pipeline é testado com diretórios temporários e sem tocar o `~/.claude` real.

## O que `tokred stats` calcula

`compute_stats` recebe **todas** as linhas de uso conhecidas, a tabela de preços, os eventos de hook, um intervalo, um agrupamento (`model`, `day` ou `project`), `now` e, opcionalmente, o instante do primeiro `configure`. Totais, grupos, economias e a taxa de cache só olham o intervalo; a projeção e a comparação antes/depois olham fora dele, de propósito.

| Número | Como |
|---|---|
| Totais e grupos | Mensagens, sessões distintas, os quatro contadores de tokens e o custo, no intervalo; grupos ordenados pela chave (grupos por dia usam dias UTC) |
| Custo real | Soma sobre as mensagens de tokens x preço (USD por milhão de tokens). Um modelo sem nenhum preço custa 0 e aparece em `unpriced_models` |
| Contrafactual "tudo no opus" | Os mesmos tokens ao preço mais recente do opus; a diferença é a **economia de roteamento**. Mensagens de modelos sem preço ficam de fora para não inflá-la. `null` se a tabela de preços não tem opus |
| Economia dos guards (**estimativa**, `estimate: true`) | Para cada bloqueio de um guard (hook `guard-*`): `bytes_saved / 4` tokens (divisão inteira por sessão) x o preço de entrada do modelo mais usado naquela sessão (por número de mensagens, empate vai para o menor id de modelo; opus se a sessão é desconhecida ou sem preço) |
| Taxa de cache do prompt | `cache_read / (cache_read + cache_write + input)`, geral e por dia UTC; `null` quando não houve tráfego de entrada |
| Projeção | Custo médio diário dos últimos 7 e dos últimos 30 dias, x 30. O divisor é a janela ou, quando os dados são mais novos que a janela, os dias desde a primeira linha (no mínimo 1), para que uma instalação recente não seja diluída |
| Antes/depois | Custo por sessão antes vs. depois do primeiro `configure` (a sessão pertence ao lado em que começou). Só aparece quando existem linhas anteriores ao primeiro `configure` |
| `unpriced_models` | Modelos precificados por **fallback de família** (preço mais recente de opus/sonnet/haiku) ou **sem resolução**, com contagem de mensagens e tokens, para o CLI sinalizar |

Só os bloqueios contam como economia dos guards: no modo `warn` a leitura acontece mesmo assim, então nada foi economizado. Tudo derivado de bytes (`bytes / 4`) é sempre rotulado como estimativa. Valores em dinheiro são arredondados a 6 casas; nenhuma razão é NaN (um período vazio dá `null`/0).

## Resumos de roteamento e guards

- **Roteamento:** classificações por classe de tarefa (o `decision` das linhas `route`, ignorando `none`), da mais frequente para a menos; delegações a partir das linhas `enforce`: `correct` (`agent_matches`), `corrected` (`agent_mismatch` com `warn` ou `deny`) e `mismatch_allowed` (`agent_mismatch` que passou).
- **Guards:** bloqueios por motivo (`reread`, `generated_file`, `large_file`, `broad_search`) com bytes evitados e tokens evitados estimados (`bytes / 4`), do maior para o menor em bytes.

## Latência adicionada pelo tokred

Os hooks rodam de forma síncrona antes de o Claude Code continuar, então `tokred stats latency` mostra por hook: execuções, p50, p95, p99 e máximo em ms; totais por sessão e por dia; a média adicionada por prompt do usuário (execuções de `route`/`UserPromptSubmit`); contagens de `ok`, `error`, `panic` e `timeout_guard`; e um alerta quando o p95 de um hook passa do orçamento de 50 ms (exatamente 50 ms está dentro do orçamento).

- Os percentis usam o método nearest-rank: o p-ésimo percentil de `n` amostras ordenadas é a de posição `ceil(p * n / 100)`. Uma única amostra é todos os percentis; empates retornam o valor empatado.
- O tempo de spawn do processo não é visível por dentro do binário, então `tokred doctor --bench` o mede de fora, e esse valor é somado a **cada** execução, marcado como `estimate: true`.
- Um auxiliar de trade-off coloca o total de ms adicionados ao lado dos tokens evitados (estimados) pelos guards e da economia total (roteamento + guards) do mesmo período.

## Com o proxy

O `stats`, o `stats latency` e o relatório HTML continuam lendo seus números dos transcripts e dos hooks, como descrito acima. Quando o [proxy](proxy.pt-BR.md) opcional está rodando, ele registra no banco seus próprios dados, exatos, por requisição: modelo pedido e modelo efetivo, tokens (incluindo gravações de cache de 1 hora), tempo até o primeiro byte e tempo total, a provável causa de um cache miss do prompt e o tempo que o próprio proxy acrescentou (o overhead, com orçamento de p95 abaixo de 10 ms). Hoje esses dados aparecem em `tokred routing status` (decisões recentes de roteamento com a economia estimada), `tokred proxy status` e `tokred suggest` (sugestões para o loop principal); mostrar os números por requisição e o overhead no `stats` e no relatório HTML está especificado na [spec](spec.pt-BR.md) e ainda não foi feito.

## Spans e intervalos

`parse_span` aceita `<n>h`, `<n>d` ou `<n>w` (n >= 1: `12h`, `7d`, `2w`, `30d`). Um span vira o intervalo semiaberto `[now - span, now + 1)`, ou seja, até o segundo atual inclusive. Passe ao `compute_stats` as linhas de `earliest_usage()` até `now`, e não só o intervalo, ou a projeção e o antes/depois ficarão errados.

## Relatório HTML

`tokred metrics` grava um arquivo HTML autocontido e offline (CSS/JS inline, gráficos SVG gerados em Rust, tema claro/escuro) em `<data>/reports/metrics.html`, sempre o mesmo arquivo, de forma atômica, e abre no navegador, a menos que `--no-open`. Seções sem dados mostram um aviso "ainda sem dados" em vez de sumirem.

Os preços vêm do JSON público do LiteLLM com um fallback embutido; um modelo desconhecido é mapeado para o preço mais recente da sua família (opus/sonnet/haiku) e sinalizado.
