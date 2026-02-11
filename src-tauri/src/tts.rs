

/// Speak text using Windows Speech API (SAPI via WinRT).
/// This is async-blocking: it waits for speech to complete.
#[cfg(target_os = "windows")]
pub fn speak_text(text: &str) -> Result<(), String> {
    use std::process::Command;

    // Use PowerShell's built-in speech synthesis for reliability
    let script = format!(
        "Add-Type -AssemblyName System.Speech; \
         $synth = New-Object System.Speech.Synthesis.SpeechSynthesizer; \
         $synth.Speak('{}');",
        text.replace('\'', "''").replace('\n', " ")
    );

    Command::new("powershell")
        .args(["-Command", &script])
        .spawn()
        .map_err(|e| format!("TTS error: {}", e))?;

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn speak_text(_text: &str) -> Result<(), String> {
    Err("TTS is only supported on Windows".to_string())
}
