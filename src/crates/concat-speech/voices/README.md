# Stock voices

The recordings Chatterbox reads a stock voice from. Chatterbox has no
speakers of its own: every voice is a few seconds of someone talking.

Each is ten seconds of a LibriVox reading, from 2:30 into its second
chapter (past LibriVox's spoken preamble). It was cut, mixed to mono,
resampled to 24 kHz, set to -18 LUFS with a constant gain (FFmpeg's
two-pass linear `loudnorm`, true peak -1.5 dBFS), and saved as 16-bit
FLAC. LibriVox readers dedicate their recordings to the public domain,
and archive.org files each of these under a public-domain licence; the
readers made them to be heard and reused. Concat credits them all the
same.

| File | Reader | Reading | Source |
|---|---|---|---|
| `elizabeth-klett.flac` | Elizabeth Klett | *Pride and Prejudice* (version 4), Jane Austen, chapter 2 | https://archive.org/details/prideandprejudice_1005_librivox (`prideandprejudice_02_austen.mp3`) |
| `mark-f-smith.flac` | Mark F. Smith | *The Adventures of Sherlock Holmes*, Arthur Conan Doyle, part 2 | https://archive.org/details/adventures_sherlockholmes_1007_librivox (`adventuresherlockholmes_02_doyle.mp3`) |
| `ruth-golding.flac` | Ruth Golding | *Wuthering Heights* (solo version), Emily Brontë, chapter 2 | https://archive.org/details/wuthering_heights_rg_librivox (`wutheringheights_02_bronte.mp3`) |
| `bob-neufeld.flac` | Bob Neufeld | *The Legend of Sleepy Hollow*, Washington Irving, part 2 | https://archive.org/details/sleepyhollow_1206_librivox (`legendofsleepyhollow_02_irving.mp3`) |

The voice list names each by its reader's first name, with an accent
only where a public source states one (see `CHATTERBOX_VOICES` in
`src/tts.rs`).
