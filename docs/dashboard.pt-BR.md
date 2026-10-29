[English](dashboard.md) | **Português**

# O dashboard ao vivo

O [proxy local](proxy.pt-BR.md) serve duas páginas na própria porta, para mostrar no navegador o que está acontecendo agora nas suas sessões do Claude Code. As duas estão embutidas no binário e não carregam nada da rede.

- **`/dashboard`**: todas as sessões ativas e as últimas 10 que ficaram ociosas ou terminaram. Abra uma para ver cada requisição que o Claude Code enviou, o que o modelo respondeu (em streaming, conforme chega), as ferramentas chamadas, o custo, os tokens, a decisão de roteamento e os subagentes em execução. Atualiza ao vivo (server-sent events).
- **`/metrics`**: o mesmo relatório HTML que o `tokred metrics` gera (últimos 30 dias, sem a auditoria de projeto).

## Como abrir

```sh
tokred dashboard
```

`tokred dashboard --no-open` imprime o endereço em vez de abrir o navegador.

O proxy precisa estar habilitado e rodando: `tokred proxy enable` o sobe, e depois `tokred claude`, o wrapper `claude` ou `tokred proxy start` o sobem de novo. O `tokred dashboard` não sobe nada e avisa se o proxy estiver fora do ar.

Outros usuários da máquina alcançam `127.0.0.1`, então as páginas pedem login. O comando obtém do proxy um **token de uso único** (usando o segredo administrativo em `<data>/proxy/secret`) e abre `http://127.0.0.1:<porta>/dashboard?token=...`. O proxy troca o token por um cookie (`HttpOnly`, `SameSite=Strict`) e redireciona para `/dashboard`. O token funciona uma vez e expira em 60 segundos; execute `tokred dashboard` de novo para um novo. Você precisa do link uma vez por navegador: o navegador guarda o cookie por 30 dias, e ele continua valendo quando o proxy reinicia, porque é assinado com uma chave derivada do segredo administrativo e traz a própria validade. Para desconectar todos os navegadores, execute `tokred proxy stop`, apague `<data>/proxy/secret` e inicie o proxy de novo: o segredo é criado outra vez e todo cookie antigo deixa de valer. Sem um cookie válido, toda página responde `401` com um aviso para executar `tokred dashboard`.

Navegadores de outros sites não conseguem usá-lo: requisições marcadas com `Sec-Fetch-Site: cross-site` ou `same-site` são recusadas (`403`), nenhum cabeçalho CORS é enviado e as páginas levam uma `Content-Security-Policy` estrita. O texto de requisições e respostas é sempre exibido como texto, nunca como HTML, porque resultados de ferramentas e saídas do modelo podem conter marcação hostil.

## Privacidade: `[proxy] log_bodies`

O que o dashboard pode mostrar depende de uma única configuração, desligada por padrão. O `tokred proxy enable` pergunta sobre ela.

| `log_bodies` | O que é mostrado e guardado |
|---|---|
| `false` (padrão) | **Só metadados**: modelos, tokens, custo, tempos, status HTTP, motivo de parada, nomes das ferramentas chamadas, decisão de roteamento, subagentes. Nada sobre o conteúdo das mensagens é lido, mantido em memória, enviado ao navegador ou guardado; as páginas avisam isso |
| `true` | Também os **conteúdos**: de cada requisição, as mensagens novas desde a requisição anterior da mesma conversa (não o histórico inteiro que é reenviado a cada volta), e a resposta (texto, raciocínio, chamadas de ferramentas com um resumo curto da entrada). Cada texto é cortado em 20.000 caracteres (`…[truncated N chars]`). Cabeçalhos e credenciais nunca são capturados |

Com `log_bodies = true`, os conteúdos ficam no banco local (tabela `captured_turns`) e só são mantidos para sessões ativas ou entre as últimas 10, e nunca por mais que `[proxy] retention_days` (padrão 90). A limpeza roda quando o proxy sobe e a cada hora; ela esvazia os conteúdos e a linha de metadados permanece até passar `retention_days`. Tudo fica na sua máquina, mas quem puder ler o banco lê os conteúdos. Mudar a configuração com `tokred proxy enable --record-content` reinicia um proxy em execução para que valha. O balanço dos prós e contras está em [decisions](decisions.pt-BR.md).

## O que ele mostra

- Uma sessão recebe o nome do jeito que o Claude Code a nomeia: o título que você deu, senão o gerado pelo Claude Code, com a pasta em que roda e um id curto. O título é lido do fim do transcript da sessão em `projects/` (do começo, quando o fim não tem) e relido enquanto a sessão roda, porque o Claude Code só dá nome à sessão depois da primeira volta. Uma sessão sem transcript mostra o id curto. Só transcripts dentro da pasta do Claude Code são lidos, e deles só se tiram o título e a pasta.
- Uma sessão está **ativa** enquanto há uma requisição em andamento ou a última foi nos últimos 5 minutos, **ociosa** até uma hora depois da última atividade, e **encerrada** depois disso.
- Um subagente conta como **em execução** enquanto uma requisição dele está em andamento e por 60 segundos depois da última.
- Os custos usam a tabela de preços do `tokred pricing`; um modelo sem preço não mostra custo.
- O detalhe de uma sessão traz as 500 voltas mais recentes; `"truncated": true` indica que há voltas mais antigas no banco.
- Requisições sem id de sessão (algumas chamadas auxiliares) não são acompanhadas.
- Uma requisição de compactação aparece como `auxiliary`.

## Referência da API

Para quem faz scripts. Todas as rotas são `GET`, respondidas pelo proxy e nunca encaminhadas à API da Anthropic, e exigem o cookie de sessão. Outros métodos respondem `405`, um caminho desconhecido sob `/dashboard/` responde `404` e um `Host` errado responde `421`.

| Caminho | Resposta |
|---|---|
| `/metrics` | O relatório HTML |
| `/dashboard`, `/dashboard/app.css`, `/dashboard/app.js` | O aplicativo de página única |
| `/dashboard/api/sessions` | `{"active": [S], "recent": [S]}`; `S.status` é `active` na primeira lista e `idle` ou `ended` na segunda |
| `/dashboard/api/sessions/{id}` | `{"session": S, "turns": [T], "truncated"?: true}`, da volta mais antiga para a mais nova; uma volta em andamento tem `in_flight: true` e o texto recebido até agora em `response.text`. Uma sessão desconhecida responde `404` |
| `/dashboard/events` (`?session=<id>`) | `text/event-stream` com os eventos abaixo e um comentário `: ping` a cada 15 segundos. Eventos não são reenviados após uma reconexão: leia a API de novo |
| `/favicon.ico`, `/dashboard/favicon.ico` | `204` |

Sessão `S`: `id`, `started_at`, `last_activity_at` (segundos unix), `status`, `requests`, `models`, `cost_usd`, `input`, `output`, `cache_read`, `cache_write`, `subagents_running`, `content_recorded`, `title` e `project` (o nome da pasta; ambos `null` quando o transcript não é encontrada).

Volta `T` (uma requisição e sua resposta): `id`, `session_id`, `ts` (segundos unix), `origin` (`main`, `subagent` ou `auxiliary`), `agent_id`, `agent_type`, `model_requested`, `model_effective`, `status`, `stop_reason`, `usage` (`input`, `output`, `cache_write`, `cache_read`), `cost_usd`, `duration_ms`, `ttfb_ms`, `tools`, `routing` (`outcome`, `reason`), `miss_cause` (por que o cache de prompt falhou, quando falhou), `series` (o início do hash do prompt de sistema: requisições de um modelo com um prompt de sistema compartilham um cache), `in_flight`, `sent` (`[{role, text}]` ou `null`), `response` (`{text, thinking, tool_uses: [{name, input_summary}]}` ou `null`).

Eventos:

| Evento | Dados |
|---|---|
| `session` | um `S` |
| `turn_start` | um `T` com `in_flight: true` e sem resposta |
| `turn_delta` | `{session_id, turn_id, text}`, para anexar à resposta (enviado só com `log_bodies = true`) |
| `turn_end` | o `T` final |
| `subagent` | `{session_id, agent_id, agent_type, state: "running" \| "finished", ts}` |
