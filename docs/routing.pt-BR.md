[English](routing.md) | **Português**

# Roteamento de modelo

Com o [proxy](proxy.pt-BR.md) rodando, o tokred pode mover para um modelo mais barato o trabalho que não precisa de um modelo caro. Uma regra molda tudo abaixo:

> **O modelo do loop principal nunca é trocado.** O modelo que você escolheu para a conversa é sempre o que a responde. O tokred só roteia requisições para as quais você não escolheu modelo, e só para um modelo mais barato.

## Quem decide o quê

| Requisição | O tokred pode trocar o modelo? |
|---|---|
| Loop principal (sua conversa) | **Nunca.** O tokred só mede e [sugere](#sugestões-para-o-loop-principal) |
| Subagente que herdou o seu modelo | Sim, uma vez, na primeira volta; a escolha vale até o fim daquele subagente |
| Subagente cujo modelo difere do loop principal (por exemplo, um definido na própria definição do agente, como fazem os agentes `tokred-*`) | Não: esse modelo foi escolhido de propósito |
| Requisição auxiliar (por exemplo, o título da sessão) | Sim, quando sai mais barato. É mantida como está quando traz marcadores `cache_control` ou ferramentas, porque pode compartilhar o cache de prompt do loop principal |
| Compactação | Nunca |

As requisições são distinguidas pelo que carregam. Requisições de subagente têm o cabeçalho `x-claude-code-agent-id`. Requisições de compactação são reconhecidas pelo cabeçalho `x-claude-code-compaction` ou `x-claude-code-request-class: compaction`. Requisições auxiliares têm `x-claude-code-request-class: auxiliary`, ou não têm id de agente, nem ferramentas, nem thinking, e têm uma única mensagem, formato que o loop principal nunca tem. Todo o resto é o loop principal.

## Como um subagente é classificado

A primeira regra que se aplica decide; na dúvida, o modelo é mantido.

1. Um subagente visto pela primeira vez depois da sua primeira volta (por exemplo, após reiniciar o proxy): mantido.
2. Um modelo que não está no catálogo de modelos do tokred: mantido.
3. Escolha explícita: um subagente cujo modelo difere do loop principal, ou cuja sessão ainda não teve nenhuma requisição do loop principal vista, é mantido.
4. O cabeçalho `x-claude-code-agent-type`, para `Explore` (tarefa de busca) e `Plan` (tarefa de planejamento).
5. Conjunto de ferramentas: só ferramentas de leitura (`Read`, `Grep`, `Glob`, `LS`, `WebFetch`, `WebSearch` e semelhantes) indicam uma tarefa de busca.
6. Palavras-chave da primeira mensagem do subagente, em inglês e em português.
7. Caso contrário: mantido.

Depois a decisão precisa passar por quatro verificações, ou o modelo é mantido: a confiança da classificação atinge `min_confidence` (padrão 0,8); o nível da classe da tarefa (`[routing.tiers]`; por padrão busca, resumo e requisições auxiliares são `cheap`, implementação é `mid` e o resto é mantido) não é `keep`; um modelo mais barato desse nível suporta a requisição; e a economia estimada é de pelo menos `min_saving_usd` (padrão 0,005).

A decisão é tomada na primeira volta do subagente e reaproveitada em todas as voltas: cada subagente monta o próprio cache de prompt, e trocar no meio descartaria raciocínio que o novo modelo não consegue ler.

## Segurança

- **Só rebaixa.** O destino precisa estar no catálogo de modelos, ser mais barato que o modelo pedido (nenhum preço maior, o preço de entrada estritamente menor), estar em `allowed_models` quando essa lista não é vazia e dar conta de tudo o que a requisição usa (modo thinking, esforço, gerenciamento de contexto, contexto de 1M e outros recursos). Uma requisição que usa algo que o tokred não reconhece é mantida.
- **Fallback.** Se a API rejeitar uma requisição reescrita (400, 404 ou 422) antes de qualquer coisa chegar ao Claude Code, a requisição original é enviada no lugar.
- **A reescrita para para um subagente** quando o modelo pedido muda no meio da execução, quando uma requisição posterior precisa de algo que o modelo mais barato não tem, ou quando a API rejeitou uma requisição reescrita. Isso é registrado como `fallback`, e o subagente não é trocado de novo.
- **Disjuntor.** Quando mais de `[routing] max_rewrite_failure_rate` (padrão 5%) das últimas 50 reescritas foram rejeitadas, a reescrita para por `[routing] cooldown` (padrão 30 minutos). Ele vive na memória do proxy: reiniciar o proxy o zera.

## Modos e comandos

| Comando | Efeito |
|---|---|
| `tokred routing status` | Modo e decisões recentes com a economia estimada |
| `tokred routing explain <id>` | Por que uma decisão foi tomada |
| `tokred routing on` | `enforce`: troca o modelo das requisições elegíveis |
| `tokred routing off` | `observe`: continua registrando o que o roteamento faria |

O padrão é `observe`: as decisões são registradas com a economia estimada e nada muda, para você conferi-las antes de ligar o roteamento. `[routing] mode = "off"` no `tokred.toml` não registra nada. Reinicie o proxy para aplicar uma mudança de modo. Chaves: `[routing]` e `[routing.tiers]` em [configuration](configuration.pt-BR.md).

## Sugestões para o loop principal

O tokred nunca troca o seu modelo principal, mas avisa quando um mais barato teria dado conta. Um prompt é simples quando levou no máximo 6 voltas do loop principal, 4.000 tokens de saída, 2.000 tokens de thinking e uma edição, e não iniciou nenhum subagente. Depois de 3 prompts simples seguidos (`[suggest] min_simple_prompts`), o tokred estima quanto o próximo modelo mais barato teria custado, incluindo reconstruir o cache de prompt uma vez, e o sugere quando a economia passa de US$ 0,05 e a sessão não teve sugestão no último `[suggest] cooldown` (padrão 6 horas).

A sugestão aparece no `tokred suggest` e no `tokred stats` e, se você escolher em `[suggest] channel`, na status line ou como uma mensagem de uma linha em um momento em que trocar sai barato: o cache de prompt do loop principal já está frio, logo depois de um `/compact`, ou o proxy não viu nenhuma requisição do loop principal na sessão. O `stats` nunca marca uma sugestão como mostrada. Trocar é sempre decisão sua (`/model`).
