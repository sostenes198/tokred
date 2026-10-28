[English](proxy.md) | **Português**

# O proxy local

O `tokred proxy` é um servidor local opcional entre o Claude Code e a API da Anthropic. Ele vê cada requisição e cada resposta. Por isso consegue medi-las com exatidão, rotear requisições de subagentes e auxiliares para um modelo mais barato, dar conselhos sobre o loop principal e manter resultados grandes de ferramentas fora do contexto sem perdê-los.

```
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:8787)──► tokred proxy ──► api.anthropic.com
```

Todo o resto do tokred (os subagentes, os hooks de guarda, `stats`, `audit`) funciona com ou sem o proxy.

## Como usar

```sh
tokred claude            # sobe o proxy se preciso e abre o Claude Code por ele, só nesta execução
tokred proxy status      # está rodando? este terminal está usando?
tokred proxy stop
```

O `tokred claude` repassa todos os argumentos ao `claude` sem alteração e sai com o código de saída do `claude`. Nada é gravado nas configurações do Claude Code. Se o proxy não subir, ou se o `ANTHROPIC_BASE_URL` já apontar para outro gateway, o Claude Code roda exatamente como antes e o tokred explica o motivo no stderr.

Para ter o proxy sempre disponível, instale-o como serviço do usuário que sobe no login (uma unit de usuário do systemd no Linux, um agente do launchd no macOS, uma tarefa de logon no Windows):

```sh
tokred proxy service install      # e depois: tokred proxy service uninstall
```

O `tokred proxy start` roda em segundo plano sem serviço; `--foreground` o mantém no terminal (o serviço usa isso).

## O que ele faz

| Recurso | Padrão | O que altera |
|---|---|---|
| Medição | sempre ligada | Nada. Registra o modelo, os tokens (inclusive gravações de cache de 1 hora), a latência e a causa provável de um cache miss de cada requisição |
| [Roteamento de modelo](routing.pt-BR.md) | `observe` | Só requisições de subagentes e auxiliares, e só em `enforce`. **O modelo do loop principal nunca é trocado** |
| Sugestões | mostradas pelo `tokred suggest` | Nada. Avisa quando os últimos prompts do loop principal foram simples o bastante para um modelo mais barato |
| Compressão reversível | `observe` | Só em `enforce`: resultados grandes de ferramentas viram uma prévia mais um id; o original é guardado e o modelo o lê de volta com o `tokred_retrieve` |
| Medição de cache de respostas | ligada | Nada. Conta repetições exatas para mostrar se um cache de respostas ajudaria |

`observe` quer dizer que o recurso registra o que *faria*, então o `tokred routing status` e o `tokred suggest` mostram a economia antes de você ligar qualquer coisa.

## Garantias

- **Fail-open.** Se qualquer etapa falhar, a requisição original é repassada sem alteração. Uma requisição reescrita que a API rejeita antes de algo chegar ao Claude Code é reenviada na forma original. O proxy nunca faz o Claude Code parar por um bug do tokred.
- **Só loopback.** Escuta em `127.0.0.1`, confere o cabeçalho `Host`, e seus endpoints de administração exigem um segredo guardado em `<dados>/proxy/secret` (permissão só do dono).
- **Repassa tudo o que não entende.** Cabeçalhos (`anthropic-beta`, `x-claude-code-*`, credenciais) e campos passam sem alteração, então recursos novos do Claude Code continuam funcionando.
- **Privado.** Credenciais e conteúdo das mensagens nunca são registrados nem guardados; só contagens, hashes e a estrutura das requisições.

## Logins

Os logins por chave de API e por assinatura claude.ai funcionam (verificado no spike F7). Com assinatura, a credencial viaja no cabeçalho `authorization` e o proxy a repassa, junto com o valor OAuth do `anthropic-beta`, sem tocar. Medir tráfego de assinatura por um gateway local é uma configuração que a documentação do Claude Code descreve. Reescrever o modelo de requisições de assinatura não é tratado pelos termos da Anthropic, então o roteamento fica em `observe` a menos que você o ligue ([decisão](decisions.pt-BR.md)).

## Limites

- O proxy conecta direto à API e confia nas raízes de certificado embutidas. Ele não lê `HTTPS_PROXY` nem um bundle de CA próprio, então não funciona atrás de um proxy corporativo que inspeciona TLS. Nesse caso use o Claude Code sem ele.
- Ele só lê o uso de uma resposta não comprimida. Pede uma ao servidor (`Accept-Encoding: identity`); se a resposta vier comprimida mesmo assim, ela é repassada sem alteração, só não é medida.
- Uma requisição de compactação não traz cabeçalho que a distinga, então pode ser tomada por um prompt simples. O roteamento nunca troca o modelo do loop principal, então isso só afeta os conselhos.
- No Windows, o `tokred proxy start` executado com a saída capturada por um pipe (por exemplo `out=$(tokred proxy start)`, ou uma etapa de CI que grava a saída) não retorna até o proxy parar: o Windows deixa o processo em segundo plano herdar o pipe. Rode-o num terminal, mande a saída para um arquivo ou `NUL`, ou use `tokred proxy service install`. O mesmo vale para o `tokred claude` quando ele precisa subir o proxy com a saída em pipe; suba o proxy antes nesse caso.
- Os registros por requisição ainda não incluem o motivo de parada, os nomes das ferramentas nem a duração de cache usada.

## Configuração

`[proxy]`, `[routing]`, `[suggest]`, `[compression]` e `[shadow]` no `tokred.toml`; veja [configuração](configuration.pt-BR.md).
