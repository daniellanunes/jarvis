# Postagem do LinkedIn — Jarvis

Anexe o vídeo `out/jarvis-linkedin.mp4` (capa: `out/capa-linkedin.jpg`) e cole o texto abaixo.
O link vai no **primeiro comentário** — o LinkedIn entrega menos posts com link no corpo.

---

## Texto da postagem

A maioria dos assistentes de IA vive numa aba do navegador. Eu queria um que morasse no meu desktop — e fizesse as coisas por mim.

Então construí o **Jarvis**: um assistente de IA para Windows que fica num "notch" discreto no topo da tela. Você fala ou digita, e ele executa:

🗓️ Cria eventos no Google Agenda — com lembrete no celular e aviso no PC
📧 Lê, resume e rascunha e-mails, mas só quando você pede
🎵 Toca a faixa exata no Spotify e controla qualquer app de mídia
🌐 Abre sites, clica e digita num navegador próprio — e confere o resultado antes de dizer que fez
📂 Encontra arquivos e anota direto no Obsidian
🎮 Mostra CPU, GPU, RAM, ping e FPS dos jogos em tempo real

Alguns desafios de engenharia que me diverti resolvendo:

→ **Nativo e leve:** núcleo em Rust com Tauri 2 e interface em TypeScript, sem framework. O instalador tem ~5 MB.
→ **Windows de verdade:** FPS via ETW (como o PresentMon), mídia via WinRT, áudio via Core Audio e o controle do navegador pelo Chrome DevTools Protocol.
→ **IA que não cai:** function calling com Gemini, Groq, OpenAI, Claude ou Ollama, com troca automática entre chaves e provedores quando um fica sem cota — sem repetir uma ação que já foi executada.
→ **Rápido:** pedidos simples ("pausa", "volume 30", "bom dia") são resolvidos na hora, sem passar pela IA.
→ **Admin uma única vez:** a permissão é pedida na instalação; depois, o app inicia elevado via Agendador de Tarefas, sem UAC a cada abertura.
→ **Confiável:** 636 testes automatizados entre Rust e TypeScript.

E o que mais me importava: **privacidade por design.** Não existe servidor do Jarvis — tudo roda local, as chaves ficam no Cofre de Credenciais do Windows, o Gmail segue o Uso Limitado das APIs do Google, e nada sai do PC (e-mail, WhatsApp, comandos do sistema) sem você tocar em "Permitir".

O projeto partiu de uma base open source (o Coucou, licença MIT) e virou um assistente completo.

É gratuito e open source (MIT). Feedback, ideias e contribuições são muito bem-vindos — o link está nos comentários. 👇

#OpenSource #Rust #Tauri #TypeScript #InteligenciaArtificial #IA #Windows #DesenvolvimentoDeSoftware #Produtividade

---

## Primeiro comentário

🔗 Código e instalador: https://github.com/daniellanunes/jarvis
📘 Guia de configuração no README do repositório.

---

## Texto alternativo do vídeo (acessibilidade)

Vídeo de apresentação do Jarvis, um assistente de IA para Windows: um notch no topo da tela mostra música, clima, internet e uso do PC; o painel aberto exibe agenda, Spotify, e-mail, arquivos e GitHub; um diagrama mostra a arquitetura em Rust e Tauri 2 com APIs nativas do Windows e provedores de IA; e telas finais destacam 636 testes automatizados, privacidade local e o link do repositório no GitHub.

---

## Dicas de publicação

- **Horário:** terça a quinta, entre 8h e 10h ou 12h e 13h.
- **Primeira hora:** responda os comentários — o LinkedIn distribui mais quando há conversa logo no início.
- **Legendas:** o vídeo já é todo em texto na tela, então funciona no mudo (como a maioria assiste no feed).
- **Versão curta:** se quiser um texto mais enxuto, mantenha o 1º parágrafo, os 6 itens com emoji, a linha de privacidade e o CTA.
