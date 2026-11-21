# Postagem do LinkedIn — Jarvis

Postagem técnica: o que o projeto faz, as ferramentas usadas e o que aprendi. Sem link nem chamada para baixar.
Anexe o vídeo `out/jarvis-linkedin.mp4` (capa: `out/capa-linkedin.jpg`) e cole o texto abaixo.

---

## Texto da postagem

A maioria dos assistentes de IA vive numa aba do navegador. Eu queria entender o que é preciso para um assistente morar no desktop — e fazer as coisas de verdade.

Então construí o Jarvis: um assistente de IA para Windows que fica num "notch" no topo da tela. Você fala ou digita, e ele executa:

🗓️ Cria eventos no Google Agenda, com lembretes
📧 Lê, resume e rascunha e-mails, só quando pedido
🎵 Toca a faixa exata no Spotify e controla qualquer app de mídia
🌐 Abre sites, clica e digita num navegador próprio, e confere o resultado
📂 Encontra arquivos e escreve notas no Obsidian
🎮 Mostra CPU, GPU, RAM, ping e FPS dos jogos em tempo real
🎙️ Reconhece a voz do usuário, no próprio PC

As ferramentas:

→ Rust + Tauri 2 no núcleo, TypeScript sem framework na interface. Instalador de ~6 MB.
→ APIs nativas do Windows: ETW para FPS (como o PresentMon), WinRT para mídia, Core Audio, PDH, Agendador de Tarefas e Credential Manager.
→ Function calling com Gemini, Groq, OpenAI, Claude e Ollama.
→ Chrome DevTools Protocol para controlar o navegador.
→ Rustpotter para detectar a palavra de ativação localmente, e Whisper para transcrever.
→ tauri-plugin-updater com instaladores assinados.

O que aprendi:

1. Voz é o problema mais difícil. Ouvir "na nuvem" o tempo todo gastou cota e disparou com vídeos e conversas. A solução foi uma cascata: um detector local treinado com 6 amostras da voz da pessoa, e uma transcrição de 2 s que confirma o nome antes de reagir. O fim da fala é medido em relação ao pico da voz, não a um limiar fixo — senão o barulho da sala mantém a gravação aberta.

2. Nem tudo precisa de IA. "Pausa", "volume 30" e "bom dia" são resolvidos por regras, na hora. A IA fica para o que é ambíguo.

3. Failover exige cuidado. Trocar de provedor quando um fica sem cota é fácil; difícil é não repetir uma ação que já foi executada antes da falha.

4. O Windows tem APIs excelentes, mas pouco documentadas. FPS via ETW e troca de saída de áudio via IPolicyConfig (uma interface não documentada) exigiram ler headers e projetos de referência.

5. Privacidade é decisão de arquitetura. Sem servidor próprio, chaves no cofre do Windows e "Permitir" antes de qualquer e-mail, mensagem ou comando do sistema.

6. Testes dão coragem para refatorar: são 649 entre Rust e TypeScript.

O projeto partiu de uma base open source (o Coucou, licença MIT) e virou um laboratório para estudar desktop nativo, áudio e agentes de IA.

#Rust #Tauri #TypeScript #InteligenciaArtificial #IA #Windows #DesenvolvimentoDeSoftware #OpenSource

---

## Texto alternativo do vídeo (acessibilidade)

Vídeo sobre o Jarvis, um assistente de IA para Windows: um notch no topo da tela mostra música, clima, internet e uso do PC; o painel aberto exibe agenda, Spotify, e-mail, arquivos e GitHub; uma tela mostra o reconhecimento da voz do usuário, a confirmação do chamado "Jarvis" e as atualizações automáticas; um diagrama mostra a arquitetura em Rust e Tauri 2 com APIs nativas do Windows e provedores de IA; e telas finais destacam 649 testes automatizados, privacidade local e as tecnologias usadas.

---

## Dicas de publicação

- **Horário:** terça a quinta, entre 8h e 10h ou 12h e 13h.
- **Primeira hora:** responda os comentários — o LinkedIn distribui mais quando há conversa logo no início.
- **Legendas:** o vídeo já é todo em texto na tela, então funciona no mudo (como a maioria assiste no feed).
- **Versão curta:** mantenha o 1º parágrafo, a lista de ferramentas e os itens 1, 2 e 5 do que aprendi.
