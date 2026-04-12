const translations = {
  pt: {
    record: "Gravar",
    history: "Histórico",
    startRecording: "Iniciar Gravação",
    stopRecording: "Parar Gravação",
    transcribe: "Transcrever",
    recording: "Gravando...",
    transcribing: "Transcrevendo... isso pode levar alguns minutos.",
    noTranscriptions: "Nenhuma transcrição ainda.",
    loading: "Carregando...",
    copyText: "Copiar texto",
    copied: "Copiado!",
    back: "Voltar",
    loadError: "Falha ao carregar transcrições",
    deleteError: "Falha ao excluir transcrição",
    meetingTitle: "Reunião",
  },
  en: {
    record: "Record",
    history: "History",
    startRecording: "Start Recording",
    stopRecording: "Stop Recording",
    transcribe: "Transcribe",
    recording: "Recording...",
    transcribing: "Transcribing... this may take a few minutes.",
    noTranscriptions: "No transcriptions yet.",
    loading: "Loading...",
    copyText: "Copy text",
    copied: "Copied!",
    back: "Back",
    loadError: "Failed to load transcriptions",
    deleteError: "Failed to delete transcription",
    meetingTitle: "Meeting",
  },
};

const lang = (navigator.language || "pt").slice(0, 2);
const locale = translations[lang] ? lang : "pt";

export function t(key) {
  return translations[locale][key] || translations.pt[key] || key;
}
