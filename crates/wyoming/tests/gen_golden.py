#!/usr/bin/env python3
"""Generate golden Wyoming fixtures from the Python reference implementation.

Run from anywhere; paths are resolved relative to this file. Requires the
reference checkout (rhasspy/wyoming @ 08a0c25) on disk; set REFERENCE_ROOT
below or pass it as the first argument.

Outputs (committed to the repo so `cargo test` needs no Python):
  tests/fixtures/golden.wyoming  - byte stream written by write_event()
  tests/fixtures/golden.json     - [{type, data, payload_hex}, ...]
"""

import json
import sys
import types
from pathlib import Path

HERE = Path(__file__).resolve().parent
REFERENCE_ROOT = Path(
    sys.argv[1]
    if len(sys.argv) > 1
    else "$HOME/Developer/forks/op6-refs/wyoming"
)

# The reference version.py reads installed package metadata; stub it so the
# bare checkout is importable.
sys.path.insert(0, str(REFERENCE_ROOT))
version_stub = types.ModuleType("wyoming.version")
version_stub.__version__ = "1.11.0"
sys.modules["wyoming.version"] = version_stub

from wyoming.asr import (  # noqa: E402
    Transcript,
    Transcribe,
    TranscriptChunk,
    TranscriptStart,
    TranscriptStop,
    VadSensitivity,
)
from wyoming.audio import AudioChunk, AudioFormat, AudioStart, AudioStop  # noqa: E402
from wyoming.error import Error  # noqa: E402
from wyoming.event import Event, write_event  # noqa: E402
from wyoming.handle import (  # noqa: E402
    Handled,
    HandledChunk,
    HandledStart,
    HandledStop,
    NotHandled,
)
from wyoming.info import (  # noqa: E402
    AsrModel,
    AsrProgram,
    Attribution,
    Describe,
    Info,
    IntentModel,
    IntentProgram,
    MicProgram,
    SndProgram,
    TtsProgram,
    TtsVoice,
    TtsVoiceSpeaker,
    WakeModel,
    WakeProgram,
    HandleModel,
    HandleProgram,
)
from wyoming.intent import Intent, Entity, IntentsStart, NotRecognized, Recognize  # noqa: E402
from wyoming.ping import Ping, Pong  # noqa: E402
from wyoming.pipeline import PipelineStage, RunPipeline  # noqa: E402
from wyoming.satellite import (  # noqa: E402
    PauseSatellite,
    RunSatellite,
    SatelliteConnected,
    SatelliteDisconnected,
    StreamingStarted,
    StreamingStopped,
)
from wyoming.snd import Played  # noqa: E402
from wyoming.timer import TimerCancelled, TimerFinished, TimerStarted, TimerUpdated  # noqa: E402
from wyoming.tts import (  # noqa: E402
    Synthesize,
    SynthesizeChunk,
    SynthesizeStart,
    SynthesizeStop,
    SynthesizeStopped,
    SynthesizeTextFormat,
    SynthesizeVoice,
)
from wyoming.user_event import UserEvent  # noqa: E402
from wyoming.vad import VoiceStarted, VoiceStopped  # noqa: E402
from wyoming.wake import Detect, Detection, NotDetected  # noqa: E402

EVENTS = [
    # audio
    AudioStart(16000, 2, 1).event(),
    AudioChunk(16000, 2, 1, bytes(range(256))).event(),
    AudioChunk(16000, 2, 1, b"\x00\x01", timestamp=1700000000).event(),
    AudioStop(timestamp=1700000001).event(),
    # vad
    VoiceStarted().event(),
    VoiceStopped(timestamp=1700000002).event(),
    # ping
    Ping().event(),
    Ping(text="ping?").event(),
    Pong(text="pong!").event(),
    # error
    Error(text="something broke").event(),
    Error(text="bad request", code="bad-request").event(),
    # timer
    TimerStarted(id="t1", total_seconds=60).event(),
    TimerStarted(
        id="t2", total_seconds=3725, name="kitchen",
        start_hours=1, start_minutes=2, start_seconds=5,
    ).event(),
    TimerUpdated(id="t1", is_active=True, total_seconds=30).event(),
    TimerCancelled(id="t1").event(),
    TimerFinished(id="t2").event(),
    # user event
    UserEvent(name="my-event").event(),
    UserEvent(name="light", data={"on": True}, context={"room": "office"}).event(),
    # snd
    Played().event(),
    # asr
    Transcript("turn on the lights").event(),
    Transcript("café — ünïcode", language="en", context={"session": "abc"}).event(),
    Transcribe(
        name="whisper",
        language="en",
        vad_sensitivity=VadSensitivity.AGGRESSIVE,
        transcript_names=["jordan"],
        transcript_terms=["laptop"],
    ).event(),
    Transcribe(vad_sensitivity="0.75").event(),
    TranscriptStart().event(),
    TranscriptChunk(text="hel").event(),
    TranscriptStop().event(),
    # wake
    Detection().event(),
    Detection(name="hey mycroft", timestamp=1700000000, speaker="jordan").event(),
    Detect().event(),
    Detect(names=["hey mycroft"]).event(),
    NotDetected().event(),
    # tts
    Synthesize(text="hello world").event(),
    Synthesize(
        text="hello",
        voice=SynthesizeVoice(name="piper", language="en", speaker="lessac"),
        text_format=SynthesizeTextFormat.SSML,
        context={"k": 1},
    ).event(),
    Synthesize(text="hi", voice=SynthesizeVoice(language="de")).event(),
    SynthesizeStart(voice=SynthesizeVoice(name="v"), text_format="markdown").event(),
    SynthesizeChunk(text="chunk").event(),
    SynthesizeStop().event(),
    SynthesizeStopped().event(),
    # intent
    Recognize(text="turn on the lights").event(),
    Intent(name="turn.on").event(),
    Intent(
        name="turn.on",
        entities=[Entity(name="light", value="desk lamp"), Entity(name="brightness")],
        text="Okay",
    ).event(),
    NotRecognized(text="sorry").event(),
    IntentsStart().event(),
    # handle
    Handled(text="done").event(),
    NotHandled().event(),
    HandledStart().event(),
    HandledChunk(text="o").event(),
    HandledStop().event(),
    # pipeline
    RunPipeline(
        start_stage=PipelineStage.WAKE, end_stage=PipelineStage.TTS, restart_on_end=True
    ).event(),
    RunPipeline(
        start_stage=PipelineStage.ASR,
        end_stage=PipelineStage.INTENT,
        wake_word_names=["hey mycroft"],
        announce_text="hello",
    ).event(),
    # satellite
    RunSatellite().event(),
    PauseSatellite().event(),
    StreamingStarted().event(),
    StreamingStopped().event(),
    SatelliteConnected().event(),
    SatelliteDisconnected().event(),
    # info
    Describe().event(),
    Info(
        asr=[
            AsrProgram(
                name="wyoming-whisper",
                attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                installed=True,
                description=None,
                version=None,
                models=[
                    AsrModel(
                        name="small",
                        attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                        installed=True,
                description=None,
                version=None,
                        languages=["en"],
                    )
                ],
            )
        ],
        tts=[
            TtsProgram(
                name="wyoming-piper",
                attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                installed=True,
                description=None,
                version=None,
                voices=[
                    TtsVoice(
                        name="en_US-lessac-medium",
                        attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                        installed=True,
                description=None,
                version=None,
                        languages=["en"],
                        speakers=[TtsVoiceSpeaker(name="lessac")],
                    )
                ],
                supports_synthesize_streaming=True,
            )
        ],
        handle=[
            HandleProgram(
                name="wyoming-hass",
                attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                installed=True,
                description=None,
                version=None,
                models=[
                    HandleModel(
                        name="default",
                        attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                        installed=True,
                description=None,
                version=None,
                        languages=["en"],
                    )
                ],
            )
        ],
        intent=[
            IntentProgram(
                name="wyoming-hassil",
                attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                installed=True,
                description=None,
                version=None,
                models=[
                    IntentModel(
                        name="default",
                        attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                        installed=True,
                description=None,
                version=None,
                        languages=["en"],
                    )
                ],
            )
        ],
        wake=[
            WakeProgram(
                name="wyoming-oww",
                attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                installed=True,
                description=None,
                version=None,
                models=[
                    WakeModel(
                        name="hey mycroft",
                        attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                        installed=True,
                description=None,
                version=None,
                        languages=["en"],
                        phrase="hey mycroft",
                    )
                ],
            )
        ],
        mic=[
            MicProgram(
                name="mic",
                attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                installed=True,
                description=None,
                version=None,
                mic_format=AudioFormat(rate=16000, width=2, channels=1),
            )
        ],
        snd=[
            SndProgram(
                name="snd",
                attribution=Attribution(name="Rhasspy", url="https://github.com/rhasspy"),
                installed=True,
                description=None,
                version=None,
                snd_format=AudioFormat(rate=22050, width=2, channels=1),
            )
        ],
    ).event(),
    # raw event with binary payload and unicode
    Event(
        type="audio-chunk",
        data={"rate": 22050, "width": 2, "channels": 1, "timestamp": None},
        payload=bytes(1024),
    ),
]


def main() -> None:
    fixtures = HERE / "fixtures"
    fixtures.mkdir(exist_ok=True)
    expected = []
    with open(fixtures / "golden.wyoming", "wb") as writer:
        for event in EVENTS:
            write_event(event, writer)
            expected.append(
                {
                    "type": event.type,
                    "data": event.data,
                    "payload_hex": event.payload.hex() if event.payload else None,
                }
            )
    with open(fixtures / "golden.json", "w") as f:
        json.dump(expected, f, ensure_ascii=False, indent=2)
    print(f"wrote {len(EVENTS)} events to {fixtures/'golden.wyoming'}")


if __name__ == "__main__":
    main()
