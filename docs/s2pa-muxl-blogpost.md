How Streamplace Works
S2PA and MUXL: Bringing Video to Content-Addressed Systems

February 20, 2026

Video is the most important media format on the web, but content-addressed systems don't really know what to do with it.

DASL gives us DRISL for deterministic serialization of structured data and CIDs for content identifiers. But MP4 files — the dominant container format for video — resist content-addressing entirely. Run the same video through ffmpeg twice with identical settings and you'll get different bytes. Different bytes means different hashes. Different hashes means no stable CID.

Meanwhile, the Coalition for Content Provenance and Authenticity (C2PA) has developed useful techniques for embedding signed provenance metadata inside media files, but their design assumes a certificate-authority model that doesn't align with decentralized identity systems. X.509 certificates. The whole PKI stack. That doesn't fit systems built on decentralized identity — atproto, IPFS, web3, anything using DIDs and secp256k1 keypairs.

We've been shipping workarounds for both problems in Streamplace since launch. Now we're formalizing them into two specifications: S2PA and MUXL.
Two specs, opposite directions

Here's the elegant part: S2PA and MUXL solve complementary problems by moving in opposite directions from C2PA.

S2PA, the Simple Standard for Provenance and Authenticity, is a superset of C2PA. It adds capabilities above C2PA: secp256k1 signing (ES256K), DID-based identity (did:key, did:plc, did:web), and verification that resolves through DID documents rather than certificate authorities. This opens C2PA's provenance model to applications that don't have — and don't want — a certificate authority.

MUXL is a strict subset of C2PA. It constrains C2PA below: a canonical form for MP4 files with deterministic behavior specified all the way down to individual atoms. Atom ordering. Timestamp bases. Chunk layout. Metadata fields. Same logical content → same bytes → same CID.

One expands the identity model upward. The other locks down the container format downward. Together they extend DASL to cover video.
The problem with MP4

MP4 is a container format, not a codec. It's a box-based structure (Apple calls them "atoms," ISO calls them "boxes") that can hold nearly anything: video, audio, subtitles, chapters, metadata.

There's no canonical ordering of atoms. No required timestamp base. Metadata fields are optional and inconsistently populated. Two muxers can produce functionally identical MP4 files — same video frames, same audio samples, same duration — that differ at the byte level. For content-addressing, this is fatal. A CID is a hash of bytes. If the bytes aren't stable, the CID isn't stable.

MUXL defines the "right answer" for all of this: a deterministic canonical form for the ISO Base Media File Format. Given the same logical content, a MUXL-compliant muxer produces identical bytes every time, on every platform.

The reference implementation will be written in Rust and compile to WASM, providing deterministic execution through the WASM 3.0 deterministic profile. The core operations are:

    Canonicalization: taking an arbitrary MP4 and producing the MUXL canonical form

    Concatenation: combining MUXL segments while preserving per-segment signatures.

    Segmentation: reversed concatenation, taking MUXL segments and producing the precise input.

These operations enable cryptographically verifiable video primitives that are maximally easy to work with. Livestreams can pass around tiny one-second MP4 files. After a six-hour stream, you have one six-hour MP4 file on your computer. Cryptographic security is preserved throughout the process. The patterns established here may also lay a trusted foundation for more radical changes to the video, such as bitexact verifiable transcoding.
The problem with C2PA

C2PA is a good idea compromised by some outdated thinking.

The Coalition — Adobe, Microsoft, Intel, BBC, others — designed a system for embedding signed provenance chains inside media files. Who created this? Who edited it? What tool was used? Each claim is signed, and the signatures chain back to a certificate authority. It's a reasonable model for institutional media: newsrooms, stock photo agencies, enterprise content management.

But it assumes you have an X.509 certificate from a recognized CA. That you're operating within the existing PKI hierarchy. That trust flows from the top down.

Decentralized systems work differently. The AT Protocol uses did:plc identifiers and secp256k1 keypairs. Bluesky users don't have certificates; they have DIDs. There's no certificate authority to appeal to — identity is cryptographic and self-sovereign.

S2PA bridges this gap. It's C2PA plus:

    ES256K signatures (ECDSA with secp256k1, per RFC 8812) instead of the RSA/ECDSA variants that require X.509

    DID-based identity as the signing identity in C2PA manifests

    Verification via DID resolution — public keys come from DID documents or AT Protocol PDS records, not certificate chains

The Streamplace fork of c2pa-rs already implements this. Every livestream segment on Streamplace gets a C2PA manifest signed with the streamer's DID. The spec work formalizes what's already in production.
What this enables

With S2PA and MUXL together, you can:

Generate stable CIDs for video. A video file can have a canonical DASL CID that doesn't depend on which encoder produced it or what platform you're on. Same content, same hash, everywhere.

Verify video authorship without CAs. A signed video can prove it came from a specific DID — and you can verify that without trusting any certificate authority, just by resolving the DID.

Content-address live streams. Streamplace already does this: each 1-second segment is a canonical MP4 with an S2PA manifest. The segments are independently verifiable and content-addressable.
Status and timeline

Both specs are in active development. S2PA is mostly documentation of existing implementation. MUXL will require more low-level video engineering work to canonicalize a "right answer" down to the level of individual atoms.

Reference implementations:

    Rust/WASM (Streamplace): primary implementation, compiles to browser and server

    C++ (MistServer): independent implementation for validation

We're also working on integration with content identification standards — perceptual hashing, semantic identification, and "soft binding" between content and external manifests in collaboration with Liccium and Hypha.

The specs will be submitted as candidate DASL specifications. The goal is a media standard that's the obvious solution for all media in decentralized social.

Streamplace is the livestreaming platform for the AT Protocol. If you're building on Bluesky, IPFS, or any content-addressed system and need video support, reach out — or just start streaming.
