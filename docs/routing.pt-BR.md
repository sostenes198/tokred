[English](routing.md) | **Português**

# Roteamento de modelo

Com o [proxy](proxy.pt-BR.md) rodando, o tokred pode mover para um modelo mais barato o trabalho que não precisa de um modelo caro. Uma regra molda tudo abaixo:

> **O modelo do loop principal nunca é trocado.** O modelo que você escolheu para a conversa é sempre o que a responde. O tokred só roteia requisições para as quais você não escolheu modelo, e só para um modelo mais barato.

## Quem decide o quê

| Requisição | O tokred pode trocar o modelo? |
|---|---|
| Loop principal (sua conversa) | **Nunca.** O tokred só mede e sugere (veja Sugestões para o loop principal, abaixo) |
| Subagente que herdou o seu modelo | Sim, uma vez, na primeira volta; a escolha vale até o fim daquele subagente |
| Subagente com modelo próprio (frontmatter, `CLAUDE_CODE_SUBAGENT_MODEL`, um agente `tokred-*`) | Não: esse modelo foi escolhido de propósito |
| Requisição auxiliar (por exemplo, o título da sessão) | Sim, quando sai mais barato, contando o custo do cache de prompt |
| Compactação | Não por padrão (`[routing] compaction = "keep"`) |

As requisições são distinguidas pelo que carregam. Requisições de subagente têm o cabeçalho `x-claude-code-agent-id`. Requisições auxiliares não têm id de agente, nem ferramentas, nem thinking, e têm uma única mensagem, formato que o loop principal nunca tem. O cabeçalho `x-claude-code-request-class` também é usado quando o Claude Code o envia.

## Como um subagente é classificado

A primeira camada que tiver confiança suficiente decide; na dúvida, o modelo é mantido.

1. Cabeçalhos, quando presentes.
2. Escolha explícita: um subagente cujo modelo difere do loop principal foi fixado de propósito e é mantido.
3. Conjunto de ferramentas: só ferramentas de leitura e busca indicam uma tarefa de busca.
4. Palavras-chave da tarefa do subagente, em inglês e em português.
5. Caso contrário: manter.

A decisão é tomada na primeira volta do subagente e reaproveitada em todas as voltas: cada subagente monta o próprio cache de prompt, e trocar no meio descartaria raciocínio que o novo modelo não consegue ler.

## Segurança

- **Só rebaixa**, e só para um modelo do catálogo que suporte tudo o que a requisição usa (modo thinking, esforço, gerenciamento de contexto, contexto de 1M). Caso contrário, o modelo é mantido.
- **Fallback.** Se a API rejeitar uma requisição reescrita (400, 404 ou 422) antes de qualquer coisa chegar ao Claude Code, a requisição original é enviada no lugar.
- **Disjuntor.** Quando rewrites demais são rejeitados, o roteamento se desliga por um tempo (`[routing] cooldown`); o `tokred routing status` mostra isso.

## Modos e comandos

| Comando | Efeito |
|---|---|
| `tokred routing status` | Modo, disjuntor e decisões recentes |
| `tokred routing explain <id>` | Por que uma decisão foi tomada |
| `tokred routing on` | `enforce`: troca o modelo das requisições elegíveis |
| `tokred routing off` | `observe`: continua registrando o que o roteamento faria |

O padrão é `observe`: as decisões são registradas com a economia estimada e nada muda, para você conferi-las antes de ligar o roteamento. Reinicie o proxy para aplicar uma mudança de modo. Chaves: `[routing]` e `[routing.tiers]` em [configuration](configuration.pt-BR.md).

## Sugestões para o loop principal

O tokred nunca troca o seu modelo principal, mas avisa quando um mais barato teria dado conta: depois de vários prompts simples seguidos (poucas voltas, saída pequena, pouca ou nenhuma edição), ele estima quanto o modelo mais barato teria custado, incluindo reconstruir o cache de prompt. A sugestão aparece no `tokred suggest` e no `tokred stats` e, se você escolher em `[suggest] channel`, na status line ou como uma mensagem de uma linha em um momento em que o cache já está frio. O `stats` nunca marca uma sugestão como mostrada. Trocar é sempre decisão sua (`/model`).
