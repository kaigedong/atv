#!/usr/bin/env python3
"""Read-only development emulator sink probe; retain statistics and a hash, not audio."""
import argparse
import base64
from contextlib import contextmanager
import hashlib
import json
import math
import os
from pathlib import Path
import struct
import secrets
import time

METHOD = "/android.emulation.control.EmulatorController/streamAudio"


def base64url(value):
    return base64.urlsafe_b64encode(value).rstrip(b"=").decode("ascii")


@contextmanager
def authorization(args):
    """Register an ephemeral public key using the emulator's supported local JWT flow."""
    bearer = os.environ.get("ATV_EMULATOR_BEARER")
    if bearer:
        yield [("authorization", "Bearer " + bearer)]
        return
    directory = args.jwks_dir
    if args.discovery_file:
        # Read only routing/public-key paths, never a stored bearer token.
        values = {}
        for line in args.discovery_file.read_text().splitlines():
            key, separator, value = line.partition("=")
            if separator and key in ("grpc.jwks", "grpc.jwk_active"):
                values[key] = value
        directory = Path(values["grpc.jwks"])
        active = Path(values.get("grpc.jwk_active", str(directory / "active.jwk")))
    else:
        active = directory / "active.jwk" if directory else None
    if not directory:
        yield []
        return
    try:
        from cryptography.hazmat.primitives import hashes
        from cryptography.hazmat.primitives.asymmetric import ec
        from cryptography.hazmat.primitives.asymmetric.utils import decode_dss_signature
    except ImportError:
        raise RuntimeError("JWT capture additionally requires cryptography in the development venv") from None
    if not directory.is_dir() or directory.stat().st_uid != os.getuid():
        raise RuntimeError("JWT key discovery must be an existing directory owned by this user")
    key = ec.generate_private_key(ec.SECP256R1())
    public = key.public_key().public_numbers()
    kid = secrets.token_hex(16)
    public_key = {"kty": "EC", "crv": "P-256", "alg": "ES256", "use": "sig", "kid": kid,
                  "x": base64url(public.x.to_bytes(32, "big")), "y": base64url(public.y.to_bytes(32, "big"))}
    registration = directory / ("atv-audio-" + kid + ".jwk")
    descriptor = os.open(registration, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w") as output:
            json.dump({"keys": [public_key]}, output)
        loaded = False
        end = time.monotonic() + 5
        while time.monotonic() < end:
            try:
                loaded = any(item.get("kid") == kid for item in json.loads(active.read_text()).get("keys", []))
            except (OSError, ValueError):
                pass
            if loaded:
                break
            time.sleep(0.05)
        if not loaded:
            raise RuntimeError("Emulator did not activate the temporary public JWT key")
        now = int(time.time())
        # The emulator's jwt-cpp validator rejects an unsolicited `typ` header.
        header = {"alg": "ES256", "kid": kid}
        claims = {"iss": "gradle-utp-emulator-control", "aud": [METHOD],
                  "iat": now - 2, "exp": now + math.ceil(args.seconds) + 20}
        signed = (base64url(json.dumps(header, separators=(",", ":")).encode()) + "."
                  + base64url(json.dumps(claims, separators=(",", ":")).encode())).encode("ascii")
        r, s = decode_dss_signature(key.sign(signed, ec.ECDSA(hashes.SHA256())))
        token = signed.decode("ascii") + "." + base64url(r.to_bytes(32, "big") + s.to_bytes(32, "big"))
        yield [("authorization", "Bearer " + token)]
    finally:
        registration.unlink(missing_ok=True)


def varint(value):
    result = bytearray()
    while value > 127:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return bytes(result)


def read_varint(data, offset):
    value = 0
    for shift in range(0, 70, 7):
        if offset >= len(data):
            raise ValueError("truncated protobuf varint")
        byte = data[offset]
        offset += 1
        if shift == 63 and byte > 1:
            raise ValueError("protobuf varint overflow")
        value |= (byte & 127) << shift
        if not byte & 128:
            return value, offset
    raise ValueError("protobuf varint overflow")


def fields(data):
    """Parse the stable numeric/bytes fields used by AudioPacket, without generated code."""
    if len(data) > 256 * 1024:
        raise ValueError("audio packet exceeds probe limit")
    result = {}
    offset = 0
    while offset < len(data):
        tag, offset = read_varint(data, offset)
        number, wire = tag >> 3, tag & 7
        if number == 0:
            raise ValueError("invalid protobuf field")
        if wire == 0:
            value, offset = read_varint(data, offset)
        elif wire == 2:
            size, offset = read_varint(data, offset)
            if size > len(data) - offset:
                raise ValueError("truncated protobuf bytes")
            value = data[offset:offset + size]
            offset += size
        elif wire in (1, 5):
            size = 8 if wire == 1 else 4
            if size > len(data) - offset:
                raise ValueError("truncated fixed protobuf field")
            offset += size
            continue
        else:
            raise ValueError("unsupported protobuf wire type")
        result[number] = value
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--endpoint", default="127.0.0.1:8554")
    parser.add_argument("--seconds", type=float, default=10)
    parser.add_argument("--output", type=Path, required=True)
    authentication = parser.add_mutually_exclusive_group()
    authentication.add_argument("--jwks-dir", type=Path,
                                help="Register a temporary public JWT key in this emulator-owned directory")
    authentication.add_argument("--discovery-file", type=Path,
                                help="Read only grpc.jwks/public-key paths from this emulator discovery file")
    args = parser.parse_args()
    host = args.endpoint.rsplit(":", 1)[0].strip("[]")
    if host not in ("127.0.0.1", "localhost", "::1"):
        parser.error("This development probe only connects to a loopback emulator")
    if not 0 < args.seconds <= 120:
        parser.error("--seconds must be in (0, 120]")
    try:
        import grpc
    except ImportError:
        parser.error("Install grpcio into a development venv before capturing")

    # AudioFormat: samplingRate=48000, channels=Stereo(1), format=AUD_FMT_S16(1).
    request = b"\x08" + varint(48000) + b"\x10\x01\x18\x01"
    digest = hashlib.sha256()
    packets = samples = nonzero = pcm_bytes = peak = 0
    energy = 0.0
    first = last = None
    formats = set()
    outcome = "deadline"
    began = time.monotonic()
    with authorization(args) as metadata, (
        grpc.secure_channel(args.endpoint, grpc.local_channel_credentials())
        if metadata else grpc.insecure_channel(args.endpoint)
    ) as channel:
        call = channel.unary_stream(
            METHOD,
            request_serializer=lambda value: value,
            response_deserializer=lambda value: value,
        )
        try:
            for encoded in call(request, timeout=args.seconds, metadata=metadata):
                packet = fields(encoded)
                audio_format = fields(packet.get(1, b""))
                rate = int(audio_format.get(1, 48000))
                channels = 2 if audio_format.get(2, 1) == 1 else 1
                encoding = int(audio_format.get(3, 1))
                if encoding != 1:
                    raise ValueError("emulator did not return signed 16-bit PCM")
                formats.add((rate, channels, encoding))
                audio = packet.get(3, b"")
                if len(audio) % (2 * channels):
                    raise ValueError("unaligned emulator PCM packet")
                digest.update(audio)
                for (sample,) in struct.iter_unpack("<h", audio):
                    samples += 1
                    nonzero += sample != 0
                    peak = max(peak, abs(sample))
                    energy += (sample / 32768.0) ** 2
                packets += 1
                pcm_bytes += len(audio)
                timestamp = int(packet.get(2, 0))
                if first is None:
                    first = timestamp
                last = timestamp
        except grpc.RpcError as error:
            outcome = error.code().name
            # Error details may contain transport metadata; never retain them.
        except ValueError:
            outcome = "INVALID_PCM_PACKET"
    result = {
        "evidence": "emulator-output-sink",
        "endpoint": args.endpoint,
        "durationWallSeconds": round(time.monotonic() - began, 3),
        "transportOutcome": outcome,
        "packets": packets,
        "pcmBytes": pcm_bytes,
        "samples": samples,
        "nonzeroSamples": nonzero,
        "peakNormalized": peak / 32768.0,
        "rmsNormalized": math.sqrt(energy / samples) if samples else 0.0,
        "pcmSha256": digest.hexdigest(),
        "firstTimestampUs": first,
        "lastTimestampUs": last,
        "formats": [dict(sampleRate=r, channels=c, encoding="S16LE") for r, c, _ in sorted(formats)],
        "nonSilentOutputObserved": packets > 0 and nonzero > 0,
    }
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    return 0 if result["nonSilentOutputObserved"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
