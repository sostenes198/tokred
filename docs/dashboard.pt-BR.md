[English](dashboard.md) | **Português**

# O painel ao vivo

O [proxy local](proxy.pt-BR.md) serve duas páginas na própria porta, no seu navegador:

- **`/dashboard`**: todas as sessões ativas agora e as últimas 10 que ficaram ociosas ou terminaram. Clique em uma para ver o que está acontecendo: cada requisição que o Claude Code enviou, o que o modelo respondeu (em streaming, conforme chega), as ferramentas chamadas, o custo, os tokens, a decisão de roteamento de modelo e os subagentes em execução. As atualizações chegam em tempo real (server-sent events).
- **`/metrics`**: o mesmo relatório HTML autocontido que o `tokred metrics` grava (últimos 30 dias, sem a auditoria do projeto).

Ambas funcionam offline: as páginas estão embutidas no binário e não carregam nada da rede.

## Como abrir

```sh
tokred dashboard            # abre o navegador
tokred dashboard --no-open  # imprime o endereço
```

O proxy precisa estar ligado e rodando: execute `tokred proxy enable` uma vez (o script de instalação o executa, a menos que você tenha passado `--no-proxy`), o que também o inicia; depois, `tokred claude`, o wrapper `claude` ou `tokred proxy start` o iniciam de novo; o `tokred dashboard` não inicia nada e avisa se o proxy está parado. Outros usuários da máquina alcançam o `127.0.0.1`, então as páginas pedem login: o comando obtém do proxy um **token de uso único** (com o segredo de administração em `<dados>/proxy/secret`) e abre `http://127.0.0.1:<porta>/dashboard?token=...`. O proxy troca o token por um cookie (`HttpOnly`, `SameSite=Strict`) e redireciona para `/dashboard`. O token do endereço **funciona uma vez e expira em 60 segundos**; rode `tokred dashboard` de novo para obter outro. Os cookies ficam na memória do proxy: reiniciá-lo encerra o seu login. Sem um cookie válido, toda página do painel responde `401` com um aviso para rodar `tokred dashboard`.

Navegadores de outros sites não conseguem usar o painel: requisições marcadas `Sec-Fetch-Site: cross-site` ou `same-site` são recusadas (`403`), nenhum cabeçalho CORS é enviado e as páginas levam uma `Content-Security-Policy` estrita (o texto do app é sempre exibido como texto, nunca como HTML, porque resultados de ferramentas e a saída do modelo podem conter marcação hostil).

## Privacidade: `[proxy] log_bodies`

O que o painel pode mostrar depende de uma opção, desligada por padrão:

| `log_bodies` | O que é mostrado e guardado |
|---|---|
| `false` (padrão) | **Só metadados**: modelos, tokens, custo, tempos, status HTTP, motivo de parada, nomes das ferramentas chamadas, decisão de roteamento, subagentes. Nada do conteúdo das mensagens é lido das requisições ou respostas, mantido em memória, enviado ao navegador ou guardado. As páginas avisam que o conteúdo não é registrado. |
| `true` | Também o **conteúdo**: de cada requisição, as mensagens novas desde a requisição anterior da mesma conversa (não o histórico inteiro, que é reenviado a cada turno), e a resposta (texto, raciocínio, chamadas de ferramenta com um resumo curto da entrada). Os textos são cortados em 20.000 caracteres (`…[truncated N chars]`). Cabeçalhos e credenciais nunca são capturados. |

Com `log_bodies = true`, o conteúdo é guardado no banco (tabela `captured_turns`) e mantido só para sessões ativas ou entre as últimas 10, e nunca por mais que `[proxy] retention_days` (padrão 90). A limpeza roda quando o proxy inicia e a cada hora; ela esvazia o conteúdo (a linha de metadados permanece até passar `retention_days`). Tudo fica na sua máquina. Veja a [decisão](decisions.pt-BR.md) sobre o compromisso.

## O que ele mostra

- Uma sessão está **ativa** enquanto há uma requisição em andamento ou a última foi há menos de 5 minutos, **ociosa** nos 55 minutos seguintes e **encerrada** depois disso.
- Um subagente conta como **em execução** enquanto há uma requisição dele em andamento e por 60 segundos após a última.
- Os custos usam a tabela de preços do `tokred pricing`; um modelo sem preço não mostra custo.
- O detalhe de uma sessão traz os 500 turnos mais recentes; `"truncated": true` indica que há turnos mais antigos no banco.
- Requisições sem id de sessão (algumas chamadas auxiliares) não são acompanhadas.

## Referência da API

Todas são `GET`, respondidas pelo proxy e nunca repassadas à API da Anthropic. Exigem o cookie de sessão.

| Caminho | Resposta |
|---|---|
| `/metrics` | O relatório HTML |
| `/dashboard`, `/dashboard/app.css`, `/dashboard/app.js` | O app de página única |
| `/dashboard/api/sessions` | `{"active": [S], "recent": [S]}`; `S.status` é `active` na primeira lista e `idle` ou `ended` na segunda |
| `/dashboard/api/sessions/{id}` | `{"session": S, "turns": [T], "truncated"?: true}`, turno mais antigo primeiro; um turno em andamento tem `in_flight: true` e o texto recebido até agora em `response.text` |
| `/dashboard/events` (`?session=<id>`) | `text/event-stream`: `session`, `turn_start`, `turn_delta`, `turn_end`, `subagent`; um comentário `: ping` a cada 15 segundos. Os eventos não são reenviados após uma reconexão: leia a API de novo |
| `/favicon.ico`, `/dashboard/favicon.ico` | `204` |

`S` (sessão): `id`, `started_at`, `last_activity_at` (segundos unix), `status`, `requests`, `models`, `cost_usd`, `input`, `output`, `cache_read`, `cache_write`, `subagents_running`, `content_recorded`.

`T` (turno, uma requisição e sua resposta): `id`, `session_id`, `ts` (segundos unix), `origin` (`main`, `subagent` ou `auxiliary`; uma requisição de compactação é `auxiliary`), `agent_id`, `agent_type`, `model_requested`, `model_effective`, `status`, `stop_reason`, `usage` (`input`, `output`, `cache_write`, `cache_read`), `cost_usd`, `duration_ms`, `ttfb_ms`, `tools`, `routing` (`outcome`, `reason`), `in_flight`, `sent` (`[{role, text}]` ou `null`), `response` (`{text, thinking, tool_uses: [{name, input_summary}]}` ou `null`).

Eventos: `session` traz um `S`; `turn_start` um `T` com `in_flight: true` e sem resposta; `turn_delta` `{session_id, turn_id, text}` para acrescentar à resposta; `turn_end` o `T` final; `subagent` `{session_id, agent_id, agent_type, state: "running" | "finished", ts}`.

Outros métodos nesses caminhos respondem `405`; um `Host` errado responde `421`.
