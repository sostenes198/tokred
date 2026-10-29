[English](proxy.md) | **Português**

# O proxy local

O `tokred proxy` é um servidor local opcional entre o Claude Code e a API da Anthropic. Ele vê cada requisição e cada resposta. Por isso consegue medi-las com exatidão, rotear requisições de subagentes e auxiliares para um modelo mais barato, dar conselhos sobre o loop principal e manter resultados grandes de ferramentas fora do contexto sem perdê-los.

```
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:1029)──► tokred proxy ──► api.anthropic.com
```

Todo o resto do tokred (os subagentes, os hooks de guarda, `stats`, `audit`) funciona com ou sem o proxy.

## Como usar

```sh
tokred claude            # sobe o proxy se preciso e abre o Claude Code por ele, só nesta execução
tokred proxy status      # está rodando? este terminal está usando?
tokred proxy stop
```

O `tokred claude` repassa todos os argumentos ao `claude` sem alteração e sai com o código de saída do `claude`. Nada é gravado nas configurações do Claude Code. Se o proxy não subir, ou se o `ANTHROPIC_BASE_URL` já apontar para outro gateway, o Claude Code roda exatamente como antes e o tokred explica o motivo no stderr.

### A chave: `[proxy] enabled`

O `enabled` no `tokred.toml` é a chave única dos recursos do proxy; o `tokred configure` a liga em uma instalação nova. O `tokred proxy enable` e o `tokred proxy disable` a alteram e valem na hora.

```sh
tokred proxy disable     # recursos desligados: a URL do tokred sai das configurações do Claude Code, mais nada muda
tokred proxy enable      # recursos ligados: sobe o proxy e aponta as configurações do Claude Code para ele
tokred initialize        # o que o wrapper de shell do `claude` executa antes de cada abertura (veja abaixo)
```

O `tokred initialize` faz o `settings.json` do Claude Code concordar com a chave, e é o que o wrapper do `claude` instalado pelo instalador executa antes de toda abertura. A única coisa que ele reconcilia é o `env.ANTHROPIC_BASE_URL` (`http://127.0.0.1:<porta>` de `[proxy] bind`):

| `enabled` | O valor está ausente ou é do tokred | O valor é outro gateway |
|---|---|---|
| `true` | Sobe o proxy se ele não estiver rodando e, quando ele responder, grava a URL. Se o proxy não puder subir, a URL é removida de novo (uma URL que aponta para um proxy morto pararia o Claude Code) e um aviso é impresso | Nada é alterado e nada é iniciado; uma linha no stderr diz que o tokred está habilitado mas o `ANTHROPIC_BASE_URL` aponta para outro lugar |
| `false` | Remove a URL do tokred (e o objeto `env` se ele ficar vazio). Um proxy em execução não é parado | Fica como está |

"A URL do tokred" é a URL atual do proxy ou qualquer URL que o tokred gravou antes (por exemplo depois de você mudar a porta); um valor exportado no shell conta como valor atual também. O seu próprio gateway nunca é sobrescrito, mesmo um em `127.0.0.1`. O `initialize` nunca falha e não imprime nada a menos que haja um aviso (`--verbose` diz o que fez, `--json` imprime um relatório); quando tudo já está certo, custa uma leitura de arquivo e uma sonda de saúde.

Enquanto a URL está no `settings.json`, o Claude Code aberto de qualquer lugar (uma IDE, um terminal sem o wrapper) passa pelo proxy. Se o proxy não estiver rodando nesse momento, a API fica inalcançável, por isso há três proteções: o serviço de usuário abaixo; o hook `session-start`, que sobe o proxy em segundo plano quando ele está habilitado, a sessão aponta para ele e ele não responde (veja [hooks](hooks.pt-BR.md)); e o `tokred doctor`, que avisa quando a URL está gravada e o proxy está parado. Para sair de qualquer problema: `tokred proxy disable` (ou `tokred unconfigure`, ou apague `ANTHROPIC_BASE_URL` do objeto `env` do `settings.json`).

Para ter o proxy sempre disponível, instale-o como serviço do usuário que sobe no login (uma unit de usuário do systemd no Linux, um agente do launchd no macOS, uma tarefa de logon no Windows):

```sh
tokred proxy service install      # e depois: tokred proxy service uninstall
```

O `tokred proxy start` roda em segundo plano sem serviço; `--foreground` o mantém no terminal (o serviço usa isso). Os dois se recusam enquanto `enabled = false`.

Com `enabled = false`, o `tokred claude` executa o `claude` direto e avisa no stderr.

## O que ele faz

| Recurso | Padrão | O que altera |
|---|---|---|
| Medição | sempre ligada | Nada. Registra o modelo, os tokens (inclusive gravações de cache de 1 hora), a latência, o motivo de parada, os nomes das ferramentas oferecidas, a duração de cache pedida (`5m` ou `1h`), o modo de roteamento em vigor e a causa provável de um cache miss de cada requisição; o `tokred stats`, o `tokred metrics` e o relatório HTML a leem como sua fonte exata ([metrics](metrics.pt-BR.md)) |
| [Roteamento de modelo](routing.pt-BR.md) | `observe` | Só requisições de subagentes e auxiliares, e só em `enforce`. **O modelo do loop principal nunca é trocado** |
| Sugestões | mostradas pelo `tokred suggest` e pelo `tokred stats` (e, se você quiser, na status line e numa mensagem de uma linha) | Nada. Avisa quando os últimos prompts do loop principal foram simples o bastante para um modelo mais barato |
| Compressão reversível | `observe` | Só em `enforce`: resultados grandes de ferramentas viram uma prévia mais um id; o original é guardado e o modelo o lê de volta com o `tokred_retrieve` |
| Medição de cache de respostas | ligada | Nada. Conta repetições exatas para mostrar se um cache de respostas ajudaria |

`observe` quer dizer que o recurso registra o que *faria*, então o `tokred routing status`, o `tokred suggest` e o `tokred stats` mostram a economia antes de você ligar qualquer coisa.

## Garantias

- **Fail-open.** Se qualquer etapa falhar, a requisição original é repassada sem alteração. Uma requisição reescrita que a API rejeita antes de algo chegar ao Claude Code é reenviada na forma original. O proxy nunca faz o Claude Code parar por um bug do tokred.
- **Só loopback.** Escuta em `127.0.0.1`, confere o cabeçalho `Host`, e seus endpoints de administração exigem um segredo guardado em `<dados>/proxy/secret` (permissão só do dono).
- **Repassa tudo o que não entende.** Cabeçalhos (`anthropic-beta`, `x-claude-code-*`, credenciais) e campos passam sem alteração, então recursos novos do Claude Code continuam funcionando.
- **Privado.** Credenciais e conteúdo das mensagens nunca são registrados nem guardados; só contagens, hashes e a estrutura das requisições.

## Logins

Os logins por chave de API e por assinatura claude.ai funcionam (verificado no spike F7). Com assinatura, a credencial viaja no cabeçalho `authorization` e o proxy a repassa, junto com o valor OAuth do `anthropic-beta`, sem tocar. Medir tráfego de assinatura por um gateway local é uma configuração que a documentação do Claude Code descreve. Reescrever o modelo de requisições de assinatura não é tratado pelos termos da Anthropic, então o roteamento fica em `observe` a menos que você o ligue ([decisão](decisions.pt-BR.md)).

## Atrás de um proxy corporativo

O proxy chega à API do mesmo jeito que o Claude Code, quando a sua rede exige.

- **Proxy HTTP.** Se `HTTPS_PROXY` estiver definida (também `ALL_PROXY` e `HTTP_PROXY`, nessa ordem, e os nomes em minúsculas), o proxy pede a ele um túnel com `CONNECT` até o host da API. `usuario:senha@` no endereço é enviado como autenticação basic e nunca é registrado em log. `NO_PROXY` funciona como no curl: nomes de host separados por vírgula (um sufixo também vale para subdomínios), endereços IP, faixas CIDR, `*`. Proxies SOCKS (`socks5://`) e `https://` não são suportados; o tokred avisa no log e no `tokred doctor` e conecta direto.
- **Autoridades certificadoras.** Além das raízes embutidas, ele confia no repositório de certificados do sistema operacional e nos arquivos PEM indicados por `NODE_EXTRA_CA_CERTS` e `SSL_CERT_FILE` (as variáveis que o Claude Code lê) e por `ca_bundle` em `[proxy]` do `tokred.toml`. É disso que um proxy que inspeciona TLS precisa.
- **Fail-open.** Um valor que não é um endereço de proxy, ou um arquivo de CA ausente ou sem certificado, é registrado uma vez (pelo nome da variável ou do arquivo, nunca pelo valor) e ignorado: o proxy conecta direto, ou com as raízes restantes. Um proxy que responde mas recusa o túnel (ou um certificado em que não se confia) resulta no `502` de sempre para um upstream inalcançável.
- O `tokred doctor` e o `tokred proxy status` mostram o proxy (só host e porta) e os arquivos de CA extras em uso.

As configurações são lidas quando o proxy sobe, do ambiente do processo que o inicia: defina as variáveis antes do `tokred proxy start` ou do `tokred claude` e, para o serviço do usuário, no ambiente em que o serviço roda (ou coloque o arquivo de CA em `ca_bundle`). Reinicie o proxy depois de mudá-las.

## Limites

- Numa rede corporativa só proxies `http://` são suportados (sem SOCKS), e as configurações de proxy são lidas uma vez, quando o proxy sobe.
- Ele só lê o uso de uma resposta não comprimida. Pede uma ao servidor (`Accept-Encoding: identity`); se a resposta vier comprimida mesmo assim, ela é repassada sem alteração, só não é medida.
- Uma requisição de compactação não traz cabeçalho que a distinga, então pode ser tomada por um prompt simples. O roteamento nunca troca o modelo do loop principal, então isso só afeta os conselhos.

## Retenção

O que o proxy guarda é apagado após `[proxy] retention_days` (padrão 90; `0` guarda tudo): uma vez quando o proxy inicia e depois a cada hora. Veja [configuração](configuration.pt-BR.md).

## Configuração

`[proxy]`, `[routing]`, `[suggest]`, `[compression]` e `[shadow]` no `tokred.toml`; veja [configuração](configuration.pt-BR.md).
