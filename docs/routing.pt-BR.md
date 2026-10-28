[English](routing.md) | **Português**

# Roteamento de modelo

Com o [proxy](proxy.pt-BR.md) rodando, o tokred pode mover para um modelo mais barato o trabalho que não precisa de um modelo caro. A regra que molda tudo abaixo:

> **O modelo do loop principal nunca é trocado.** O modelo que você escolheu para a conversa é sempre o que a responde. O tokred só roteia requisições para as quais você não escolheu modelo, e só para um modelo mais barato.

## Quem decide o quê

| Requisição | O tokred pode trocar o modelo? |
|---|---|
| Loop principal (sua conversa) | **Nunca.** O tokred só mede e [sugere](#sugestões-para-o-loop-principal) |
| Subagente que herdou o seu modelo | Sim, uma vez, na primeira volta, e a escolha vale até o fim daquele subagente |
| Subagente com modelo próprio (frontmatter, `CLAUDE_CODE_SUBAGENT_MODEL`, um agente `tokred-*`) | Não: esse modelo foi escolhido de propósito |
| Requisição auxiliar (ex.: o título da sessão) | Sim, quando sai mais barato, contando o custo do cache de prompt |
| Compactação | Não (a qualidade do resumo afeta o resto da sessão) |

Como o tokred distingue: requisições de subagente levam o cabeçalho `x-claude-code-agent-id`. Requisições auxiliares não têm ferramentas, nem thinking, e têm uma única mensagem, formato que o loop principal nunca tem. Quando um Claude Code futuro enviar o cabeçalho documentado `x-claude-code-request-class`, ele também será usado.

## Como um subagente é classificado

A primeira camada confiante o bastante decide; na dúvida, o modelo é mantido.

1. Cabeçalhos, quando existem (tipo de agente).
2. Escolha explícita: um subagente cujo modelo difere do modelo do loop principal foi fixado de propósito e é mantido.
3. Conjunto de ferramentas: só ferramentas de leitura e busca indica uma tarefa de busca.
4. Palavras-chave da tarefa do subagente, em inglês e português.
5. Caso contrário: mantém.

A decisão é tomada na primeira volta do subagente e reaproveitada em todas as outras: cada subagente monta o próprio cache de prompt, e trocar no meio descartaria um raciocínio que o novo modelo não consegue ler.

## Segurança

- **Só para baixo**, e só para um modelo do catálogo que tenha todas as capacidades que a requisição usa (modo de thinking, esforço, gerenciamento de contexto, contexto de 1M). Caso contrário, o modelo é mantido.
- **Fallback:** se a API rejeitar uma requisição reescrita antes de algo chegar ao Claude Code, a original é enviada no lugar.
- **Disjuntor:** rejeições demais desligam o roteamento por um tempo; o `tokred routing status` mostra isso.

## Modos

```sh
tokred routing status            # modo, disjuntor, decisões recentes
tokred routing explain <id>      # por que uma decisão foi tomada
tokred routing on                # enforce: troca o modelo das requisições elegíveis
tokred routing off               # observe: continua registrando o que o roteamento faria
```

O padrão é `observe`: as decisões ficam registradas com a economia estimada e nada muda, para você conferi-las antes de ligar o roteamento.

## Sugestões para o loop principal

O tokred nunca troca o seu modelo principal, mas avisa quando um mais barato teria dado conta: depois de vários prompts simples seguidos (poucas voltas, pouca saída, pouca ou nenhuma edição), ele estima quanto o modelo mais barato teria custado, incluindo reconstruir o cache de prompt, e mostra a sugestão no `tokred suggest`, no `tokred stats` e, se você quiser, na status line ou numa mensagem de uma linha num momento em que o cache já esfriou. A troca é sempre decisão sua (`/model`).

## Configuração

`[routing]`, `[routing.tiers]` e `[suggest]` no `tokred.toml`; veja [configuração](configuration.pt-BR.md).
