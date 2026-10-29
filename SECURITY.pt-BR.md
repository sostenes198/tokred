[English](SECURITY.md) | **Português**

# Política de segurança

Como reportar uma vulnerabilidade no tokred e como o tokred foi projetado para limitar o estrago de uma.

## Versões suportadas

Apenas a última release recebe correções de segurança. Atualize com `tokred update` (veja [instalação](docs/installation.pt-BR.md)).

## Reportar uma vulnerabilidade

Use o relato privado do GitHub: abra a aba **Security** do repositório e escolha **Report a vulnerability**, ou acesse <https://github.com/sostenes198/tokred/security/advisories/new>. Não abra uma issue pública com detalhes de exploração.

Inclua, por favor:

- a versão do tokred (`tokred --version`), seu sistema operacional e como você instalou;
- o que uma pessoa mal-intencionada consegue fazer e os passos para reproduzir;
- uma prova de conceito, se tiver, sem segredos reais.

Você pode esperar uma confirmação de recebimento do seu relato, uma correção em uma nova release e crédito no changelog e no aviso, se quiser. Pedimos que nos dê tempo para lançar a correção antes de divulgar o problema publicamente.

## Modelo de segurança

- **Somente local.** O proxy escuta em `127.0.0.1` e rejeita (com `421`) qualquer requisição cujo cabeçalho `Host` não seja um nome de loopback na porta do próprio proxy, o que bloqueia ataques de DNS rebinding vindos de páginas web.
- **Endpoints administrativos** (`/__tokred/status`, `/__tokred/shutdown`, `/__tokred/dashboard-token`) exigem um segredo guardado em um arquivo na pasta de dados que só o dono consegue ler (modo `0600` no macOS e no Linux).
- **O dashboard** não pode ser aberto por outro usuário local: `tokred dashboard` pede ao proxy um token de uso único (válido por 60 segundos, usável uma vez) e o navegador o troca por um cookie `HttpOnly` e `SameSite=Strict`.
- **Credenciais** (`Authorization`, `x-api-key`) são encaminhadas à API da Anthropic e nunca são guardadas nem registradas em log.
- **O conteúdo das mensagens** só é guardado quando você ativa `[proxy] log_bodies`, que vem desligado por padrão.
- **Os hooks são fail-open:** diante de qualquer erro, pânico ou timeout eles saem com código 0 e sem saída, então o tokred não consegue bloquear o Claude Code.
- **Os downloads são verificados:** o instalador e o `tokred update` conferem o checksum SHA-256 do arquivo antes de gravar qualquer coisa.
- **Desinstalação:** `tokred uninstall` grava seu script final de limpeza em um arquivo novo, com nome único e legível apenas pelo dono, e não em um nome fixo em uma pasta compartilhada.

Veja [proxy](docs/proxy.pt-BR.md) e [dashboard](docs/dashboard.pt-BR.md) para os detalhes.
