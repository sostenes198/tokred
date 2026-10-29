[English](proxy.md) | **Português**

# O proxy local

O proxy é um servidor local opcional entre o Claude Code e a API da Anthropic: ele mede cada requisição com exatidão, pode rotear requisições de subagentes e auxiliares para um modelo mais barato, dá conselhos sobre o loop principal e pode manter resultados grandes de ferramentas fora do contexto sem perdê-los.

```text
Claude Code ──(ANTHROPIC_BASE_URL=http://127.0.0.1:1029)──► tokred proxy ──► api.anthropic.com
```

O resto do tokred (os [hooks e agentes](hooks-and-agents.pt-BR.md), `stats`, `audit`) funciona com ou sem ele.

## Comandos

| Comando | O que faz |
|---|---|
| `tokred proxy enable` | Liga os recursos do proxy: escolhe a porta e a gravação de conteúdo, salva, sobe o proxy e aponta as configurações do Claude Code para ele |
| `tokred proxy disable` | Desliga os recursos e remove a `ANTHROPIC_BASE_URL` do tokred das configurações do Claude Code. Um proxy em execução não é parado |
| `tokred proxy start` | Sobe o proxy em segundo plano. `--foreground` o mantém no terminal (o serviço de usuário usa isso); `--bind HOST:PORT` substitui o `[proxy] bind` nesta execução. Recusa enquanto os recursos estão desligados |
| `tokred proxy stop` | Para o proxy e espera até ele terminar |
| `tokred proxy status` | Está rodando, em qual porta, e este terminal o usa (`--json` para scripts) |
| `tokred proxy service install` / `uninstall` | Instala o proxy como serviço de usuário que sobe no login (unit de usuário do systemd, agente do launchd, tarefa de logon no Windows), ou o remove. O `install` recusa enquanto os recursos estão desligados |
| `tokred claude [args]` | Sobe o proxy se preciso e executa o `claude` por ele, só nesta execução. Argumentos e código de saída passam direto (coloque `--` antes para repassar `-h`, `--help`, `-V` ou `--version` como primeiro argumento: o tokred responde a esses por conta própria); nada é gravado nas configurações do Claude Code. Se o proxy não conseguir subir, se a `ANTHROPIC_BASE_URL` já apontar para outro gateway ou se os recursos estiverem desligados, executa o `claude` direto e diz o motivo no stderr |
| `tokred dashboard` | Abre o [dashboard ao vivo](dashboard.pt-BR.md) |

O [script de instalação](installation.pt-BR.md) executa `tokred proxy enable` (a menos que você passe `--no-proxy`) e instala o wrapper de shell `claude`, que executa `tokred initialize` antes de cada abertura. O `tokred configure` só prepara hooks e agentes e nunca mexe no proxy.

### `tokred proxy enable`

| Flag | Significado |
|---|---|
| `--port <n>` (ou `TOKRED_PORT`) | Porta do `[proxy] bind`, de 1024 a 65535 (o host do `bind` permanece, `127.0.0.1` por padrão). Perguntada em um terminal (padrão: a porta salva, senão 1029; o comando verifica se a porta está livre e aceita a que o proxy desta instalação já usa). Uma porta ocupada informada por flag só avisa |
| `--record-content <true\|false>` (ou `TOKRED_RECORD_CONTENT`) | `[proxy] log_bodies`: se o [dashboard](dashboard.pt-BR.md) pode mostrar o que foi enviado e devolvido. Perguntada em um terminal (padrão: a resposta salva, senão sim). Sem terminal, uma resposta não salva é não |
| `--yes`, `-y` | Não pergunta nada: usa as flags, os valores salvos, senão porta 1029 e sem gravação |

Uma flag vale mais que a variável de ambiente correspondente, e suas outras chaves no `tokred.toml` permanecem. Se o proxy já estiver rodando em outra porta, ou com outra configuração de gravação, ele é reiniciado para aplicar a mudança.

### A chave: `[proxy] enabled`

O `enabled` no `tokred.toml` é a chave única dos recursos do proxy. Ela é `false` até você executar `tokred proxy enable`.

O `tokred initialize` faz o `settings.json` do Claude Code concordar com ela, e é o que o wrapper `claude` executa antes de cada abertura. Ele reconcilia uma única chave, `env.ANTHROPIC_BASE_URL` (`http://127.0.0.1:<porta>`, a partir do `[proxy] bind`):

| `enabled` | O valor está ausente ou é do tokred | O valor é de outro gateway |
|---|---|---|
| `true` | Sobe o proxy se ele não estiver rodando e, quando responde, grava a URL. Se não conseguir subir, a URL é removida de novo (uma URL apontando para um proxy morto impediria o Claude Code de funcionar) e um aviso é impresso | Nada é alterado nem iniciado; uma linha no stderr avisa |
| `false` | Remove a URL do tokred. Um proxy em execução não é parado | Deixada como está |

"A URL do tokred" é a URL atual do proxy ou qualquer URL que o tokred gravou antes (por exemplo, após mudar a porta). O seu gateway nunca é sobrescrito, nem mesmo um em `127.0.0.1`. O `initialize` nunca falha e não imprime nada, a não ser que haja um aviso (`--verbose` diz o que fez, `--json` imprime um relatório).

Enquanto a URL está no `settings.json`, o Claude Code aberto de qualquer lugar (uma IDE, um terminal sem o wrapper) passa pelo proxy, e se o proxy estiver fora do ar ele não alcança a API. Três salvaguardas cobrem isso: o serviço de usuário, o hook `session-start` (sobe o proxy em segundo plano quando a sessão aponta para ele e ele não responde) e o `tokred doctor` (avisa quando a URL está gravada e o proxy está fora do ar). Para sair de qualquer problema, execute `tokred proxy disable` (ou `tokred uninstall`, ou apague `ANTHROPIC_BASE_URL` do objeto `env` do `settings.json`).

## O que ele faz

| Recurso | Padrão | O que altera |
|---|---|---|
| Medição | sempre ligada | Nada. Registra o modelo, os tokens (incluindo gravações de cache de 1 hora), a latência, o motivo de parada, os nomes das ferramentas, a duração do cache, o modo de roteamento em vigor e a causa provável de uma falha de cache de prompt. `tokred stats`, `tokred metrics` e o relatório os usam como fonte exata ([metrics](metrics.pt-BR.md)) |
| [Roteamento de modelo](routing.pt-BR.md) | `observe` | Só requisições de subagentes e auxiliares, e só em `enforce`. **O modelo do loop principal nunca é trocado** |
| Sugestões | mostradas pelo `tokred suggest` e pelo `tokred stats` | Nada. Avisa quando os últimos prompts do loop principal eram simples o bastante para um modelo mais barato |
| Compressão reversível | `observe` | Só em `enforce`: resultados grandes de ferramentas são trocados por uma prévia e um id; o original é guardado e o modelo o relê com `tokred_retrieve`. Ligue com `tokred compression on` e reinicie o proxy e o Claude Code |
| Medição de cache de respostas | ligada | Nada. Conta repetições exatas para mostrar se um cache de respostas ajudaria |

`observe` significa que o recurso registra o que faria, então `tokred routing status`, `tokred suggest` e `tokred stats` mostram a economia antes de você ligar qualquer coisa. As chaves estão em [configuration](configuration.pt-BR.md).

## Garantias

- **Fail-open.** Se qualquer etapa falhar, a requisição original é encaminhada sem alteração. Uma requisição reescrita que a API rejeita (400, 404 ou 422) antes de qualquer coisa chegar ao Claude Code é enviada de novo na forma original. Um bug do tokred nunca impede o Claude Code de funcionar.
- **Só loopback.** Ele se recusa a escutar em um endereço que não seja loopback, só responde a requisições cujo cabeçalho `Host` nomeia um endereço loopback e a sua própria porta (a defesa contra DNS rebinding), e seus endpoints administrativos (`/__tokred/...`, exceto `health`) exigem um segredo guardado em `<data>/proxy/secret`, com permissão só do dono.
- **Nada que ele não entenda é alterado.** Cabeçalhos (`anthropic-beta`, `x-claude-code-*`, credenciais) e campos passam sem alteração, então recursos novos do Claude Code continuam funcionando. Além de descartar os cabeçalhos hop-by-hop, o único cabeçalho que ele reescreve é o `Accept-Encoding` (quando o cliente envia um), enviado como `identity` ao upstream para que a resposta possa ser lida enquanto chega em streaming.
- **O modelo do loop principal nunca é trocado**, seja qual for a configuração.
- **Privado.** Credenciais nunca são registradas nem guardadas. O texto das mensagens só é guardado se você ligar `[proxy] log_bodies` (veja [dashboard](dashboard.pt-BR.md)) ou colocar a compressão em `enforce`, que guarda o original de cada resultado de ferramenta substituído.

## Logins

Tanto o login por chave de API quanto o da assinatura claude.ai funcionam. Com assinatura, a credencial viaja no cabeçalho `authorization`, e o proxy a encaminha, junto com o valor OAuth do `anthropic-beta`, sem alteração. A própria documentação do Claude Code descreve medir o tráfego de assinatura por um gateway local.

Os termos da Anthropic não tratam da reescrita do modelo das requisições de assinatura, então o roteamento fica em `observe` por padrão para todo login. O `tokred doctor` imprime uma nota sempre que `enforce` está ligado. Ligá-lo é uma escolha informada sua ([decisions](decisions.pt-BR.md)).

## Atrás de uma rede corporativa

O proxy alcança a API do mesmo jeito que o Claude Code.

| Configuração | Efeito |
|---|---|
| `HTTPS_PROXY`, senão `ALL_PROXY`, senão `HTTP_PROXY` (cada um também em minúsculas) | O proxy pede a esse proxy HTTP um túnel `CONNECT` até o host da API (só para um upstream `https`). `usuario:senha@` é enviado como autenticação básica e nunca registrado |
| `NO_PROXY` (ou `no_proxy`) | Como no curl: nomes de host separados por vírgula (um sufixo também vale para subdomínios), endereços IP, faixas CIDR, `*` |
| `NODE_EXTRA_CA_CERTS`, `SSL_CERT_FILE`, `[proxy] ca_bundle` | Arquivos PEM com autoridades certificadoras extras, confiadas além das raízes embutidas e do repositório de certificados do sistema operacional. É o que um proxy que inspeciona TLS exige |

Um valor de proxy que não é endereço ou que usa `socks5://` ou `https://`, e um arquivo de CA ausente ou vazio, são registrados uma vez (pelo nome, nunca pelo valor) e ignorados: o proxy conecta direto, ou com as raízes restantes. Um proxy que recusa o túnel, ou um certificado não confiável, resulta no `502` de sempre. O `tokred doctor` e o `tokred proxy status --json` mostram o proxy (só host e porta) e os arquivos de CA extras em uso.

As configurações são lidas uma vez, quando o proxy sobe, a partir do ambiente do processo que o inicia. Defina as variáveis antes de `tokred proxy start` ou `tokred claude` (no caso do serviço de usuário, no ambiente em que o serviço roda, ou use `ca_bundle`) e reinicie o proxy depois de alterá-las.

## Retenção

O que o proxy guarda é apagado depois de `[proxy] retention_days` (padrão 90), uma vez quando o proxy sobe e depois a cada hora. `0` mantém todas as linhas; o texto gravado pelo dashboard ainda é esvaziado nas sessões que não estão ativas nem entre as 10 últimas ([dashboard](dashboard.pt-BR.md)). Resultados de ferramentas guardados para o `tokred_retrieve` e as substituições de uma conversa que ainda tem uma requisição dentro do período não são apagados por essa limpeza, qualquer que seja a idade.

## Limites

- Só há suporte a proxies `http://` (sem SOCKS, sem TLS até o proxy).
- O uso só é lido de uma resposta sem compressão. Se uma resposta chegar comprimida mesmo assim, ela é encaminhada sem alteração e não é medida.
- Uma requisição de compactação só é reconhecida quando o Claude Code envia `x-claude-code-compaction` ou `x-claude-code-request-class: compaction`; a versão testada não enviou nenhum dos dois, então essa requisição pode ser tomada por um prompt simples. O roteamento nunca troca o modelo do loop principal, então isso só afeta o conselho.
