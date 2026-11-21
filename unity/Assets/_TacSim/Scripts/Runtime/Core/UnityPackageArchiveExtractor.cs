using System;
using System.Collections.Generic;
using System.IO;
using System.IO.Compression;
using System.Text;

namespace TacSim.Core
{
    /// <summary>
    /// Deterministic synchronous extractor for Unity <c>.unitypackage</c> (<c>.tar.gz</c>) archives.
    /// Ensures all <c>Assets/...</c> payloads and <c>.meta</c> GUID sidecars exist on disk before
    /// <c>AssetDatabase.Refresh(ImportAssetOptions.ForceSynchronousImport)</c> runs, avoiding
    /// <c>Package.Import</c>'s asynchronous decompression race under <c>-batchmode -quit</c>.
    /// </summary>
    public static class UnityPackageArchiveExtractor
    {
        public const int MaxEntries = 32768;
        public const int MaxMetaHeaderBytes = 64 * 1024;
        public const long MaxSingleEntryBytes = 256L * 1024 * 1024;
        public const long MaxTotalUncompressedBytes = 2L * 1024 * 1024 * 1024;

        private static readonly HashSet<string> BlockedExecutableExtensions = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            ".cs", ".asmdef", ".asmref", ".dll", ".exe", ".so", ".dylib", ".bundle",
            ".rsp", ".js", ".boo", ".shader", ".compute", ".raytrace", ".cginc", ".hlsl",
            ".glslinc", ".shadergraph", ".shadersubgraph", ".bat", ".cmd", ".ps1", ".sh",
            ".py", ".rb", ".pl", ".vbs", ".wsf", ".hta", ".msi", ".com", ".scr", ".jar",
            ".aar", ".winmd", ".pdb", ".mdb", ".a", ".lib", ".ocx", ".meta"
        };

        private static readonly HashSet<string> AllowedPassiveAssetExtensions = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            // Meshes / 3D models
            ".fbx", ".obj", ".dae", ".3ds", ".blend", ".mesh", ".ply", ".stl", ".glb", ".gltf",
            // Textures / images
            ".png", ".tga", ".jpg", ".jpeg", ".tif", ".tiff", ".psd", ".exr", ".hdr",
            ".bmp", ".gif", ".dds", ".ktx", ".webp", ".cubemap",
            // Materials / prefabs / animations / passive Unity assets
            ".mat", ".prefab", ".asset", ".anim", ".controller", ".overridecontroller",
            ".mask", ".avatar", ".physicmaterial", ".physicsmaterial", ".physicsmaterial2d",
            ".mixer", ".rendertexture", ".lighting", ".flare", ".guiskin", ".fontsettings",
            ".ttf", ".otf", ".unity", ".unitypackage", ".playable", ".signal",
            ".terrainlayer", ".brush", ".preset", ".spriteatlas", ".spriteatlasv2",
            // Audio
            ".wav", ".mp3", ".ogg", ".aif", ".aiff", ".flac",
            // Passive docs / data
            ".txt", ".md", ".pdf", ".json", ".xml", ".csv", ".bytes"
        };

        private static readonly HashSet<string> WindowsReservedDeviceNames = new HashSet<string>(StringComparer.OrdinalIgnoreCase)
        {
            "CON", "PRN", "AUX", "NUL",
            "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8", "COM9",
            "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9"
        };

        public static int ExtractPackageFile(string packageFilePath, string projectRootDir)
        {
            if (string.IsNullOrWhiteSpace(packageFilePath) || !File.Exists(packageFilePath))
                throw new FileNotFoundException("Unity package file not found.", packageFilePath);
            using (var fs = new FileStream(packageFilePath, FileMode.Open, FileAccess.Read, FileShare.Read))
            {
                return ExtractPackageStream(fs, projectRootDir);
            }
        }

        public static int ExtractPackageStream(Stream seekableGzipTarStream, string projectRootDir)
        {
            return ExtractPackageStream(seekableGzipTarStream, projectRootDir, MaxSingleEntryBytes, MaxTotalUncompressedBytes);
        }

        public static int ExtractPackageStream(
            Stream seekableGzipTarStream,
            string projectRootDir,
            long maxSingleEntryBytes,
            long maxTotalUncompressedBytes)
        {
            if (seekableGzipTarStream == null) throw new ArgumentNullException(nameof(seekableGzipTarStream));
            if (string.IsNullOrWhiteSpace(projectRootDir)) throw new ArgumentNullException(nameof(projectRootDir));
            if (maxSingleEntryBytes <= 0) throw new ArgumentOutOfRangeException(nameof(maxSingleEntryBytes));
            if (maxTotalUncompressedBytes <= 0) throw new ArgumentOutOfRangeException(nameof(maxTotalUncompressedBytes));

            string fullProjectRoot = Path.GetFullPath(projectRootDir);
            string fullAssetsRoot = Path.GetFullPath(Path.Combine(fullProjectRoot, "Assets"));
            string assetsPrefix = fullAssetsRoot.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar) + Path.DirectorySeparatorChar;

            // Pass 1: Collect `<guid>/pathname` mappings (handles archives where `<guid>/asset` precedes `<guid>/pathname`).
            seekableGzipTarStream.Position = 0;
            var guidToRelPath = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            var folderGuidsWithPayload = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            using (var gz1 = new GZipStream(seekableGzipTarStream, CompressionMode.Decompress, leaveOpen: true))
            {
                WalkTarEntries(gz1, maxSingleEntryBytes, maxTotalUncompressedBytes, (entryPath, typeFlag, size, dataStream) =>
                {
                    if (!TrySplitGuidEntry(entryPath, out string guid, out string leaf))
                    {
                        SkipExact(dataStream, size);
                        return;
                    }

                    if (string.Equals(leaf, "pathname", StringComparison.OrdinalIgnoreCase) && size > 0 && size <= 4096)
                    {
                        byte[] buf = ReadExact(dataStream, (int)size);
                        string raw = Encoding.UTF8.GetString(buf);
                        int nl = raw.IndexOfAny(new[] { '\r', '\n', '\0' });
                        if (nl >= 0) raw = raw.Substring(0, nl);
                        string cleaned = raw.Replace('\\', '/');
                        if (TryResolveSafeAssetDestination(fullProjectRoot, assetsPrefix, cleaned, out _, allowExtensionlessDirectory: true))
                            guidToRelPath[guid] = cleaned;
                    }
                    else
                    {
                        if (string.Equals(leaf, "asset", StringComparison.OrdinalIgnoreCase))
                        {
                            folderGuidsWithPayload.Add(guid);
                        }
                        else if (string.Equals(leaf, "asset.meta", StringComparison.OrdinalIgnoreCase) && size > MaxMetaHeaderBytes)
                        {
                            throw new InvalidDataException($"asset.meta size {size} exceeds permitted limit ({MaxMetaHeaderBytes} bytes).");
                        }
                        SkipExact(dataStream, size);
                    }
                });
            }

            // Re-validate every GUID carrying a file payload against strict passive-asset extension allowlist
            // so extensionless or disallowed payloads (and their sidecar .meta) are never written.
            var disallowedFileGuids = new List<string>();
            foreach (string payloadGuid in folderGuidsWithPayload)
            {
                if (guidToRelPath.TryGetValue(payloadGuid, out string relPath) &&
                    !TryResolveSafeAssetDestination(fullProjectRoot, assetsPrefix, relPath, out _, allowExtensionlessDirectory: false))
                {
                    disallowedFileGuids.Add(payloadGuid);
                }
            }
            for (int i = 0; i < disallowedFileGuids.Count; i++)
                guidToRelPath.Remove(disallowedFileGuids[i]);

            // First ensure all folder entries (GUIDs that have a pathname & meta, but no `asset` file payload) exist as directories.
            foreach (var kv in guidToRelPath)
            {
                if (!folderGuidsWithPayload.Contains(kv.Key) &&
                    TryResolveSafeAssetDestination(fullProjectRoot, assetsPrefix, kv.Value, out string folderFull, allowExtensionlessDirectory: true))
                {
                    Directory.CreateDirectory(folderFull);
                }
            }

            // Pass 2: Stream `<guid>/asset` and `<guid>/asset.meta` directly to disk.
            seekableGzipTarStream.Position = 0;
            int filesWritten = 0;
            byte[] copyBuf = new byte[65536];

            using (var gz2 = new GZipStream(seekableGzipTarStream, CompressionMode.Decompress, leaveOpen: true))
            {
                WalkTarEntries(gz2, maxSingleEntryBytes, maxTotalUncompressedBytes, (entryPath, typeFlag, size, dataStream) =>
                {
                    if (!TrySplitGuidEntry(entryPath, out string guid, out string leaf) ||
                        !guidToRelPath.TryGetValue(guid, out string relAssetPath))
                    {
                        SkipExact(dataStream, size);
                        return;
                    }

                    bool isAssetFile = string.Equals(leaf, "asset", StringComparison.OrdinalIgnoreCase);
                    bool isAssetMeta = string.Equals(leaf, "asset.meta", StringComparison.OrdinalIgnoreCase);
                    if (!isAssetFile && !isAssetMeta)
                    {
                        SkipExact(dataStream, size);
                        return;
                    }

                    bool allowExtensionless = !isAssetFile && !folderGuidsWithPayload.Contains(guid);
                    if (!TryResolveSafeAssetDestination(fullProjectRoot, assetsPrefix, relAssetPath, out string destAssetFull, allowExtensionless))
                    {
                        SkipExact(dataStream, size);
                        return;
                    }

                    if (isAssetFile)
                    {
                        string parentDir = Path.GetDirectoryName(destAssetFull);
                        if (!string.IsNullOrEmpty(parentDir))
                            Directory.CreateDirectory(parentDir);
                        using (var outFs = new FileStream(destAssetFull, FileMode.Create, FileAccess.Write, FileShare.None))
                        {
                            CopyExact(dataStream, outFs, size, copyBuf);
                        }
                        filesWritten++;
                    }
                    else
                    {
                        if (size > MaxMetaHeaderBytes)
                            throw new InvalidDataException($"asset.meta size {size} exceeds permitted limit ({MaxMetaHeaderBytes} bytes).");
                        string metaFull = destAssetFull + ".meta";
                        string parentDir = Path.GetDirectoryName(metaFull);
                        if (!string.IsNullOrEmpty(parentDir))
                            Directory.CreateDirectory(parentDir);
                        using (var outFs = new FileStream(metaFull, FileMode.Create, FileAccess.Write, FileShare.None))
                        {
                            CopyExact(dataStream, outFs, size, copyBuf);
                        }
                        filesWritten++;
                    }
                });
            }

            return filesWritten;
        }

        private static bool IsWindowsReservedName(string segment)
        {
            if (string.IsNullOrEmpty(segment))
                return true;
            string stem = segment;
            int dot = stem.IndexOf('.');
            if (dot >= 0)
                stem = stem.Substring(0, dot);
            stem = stem.TrimEnd(' ', '.');
            return WindowsReservedDeviceNames.Contains(stem);
        }

        internal static bool TryResolveSafeAssetDestination(string fullProjectRoot, string assetsPrefix, string relAssetPath, out string fullDest)
        {
            return TryResolveSafeAssetDestination(fullProjectRoot, assetsPrefix, relAssetPath, out fullDest, allowExtensionlessDirectory: false);
        }

        internal static bool TryResolveSafeAssetDestination(
            string fullProjectRoot,
            string assetsPrefix,
            string relAssetPath,
            out string fullDest,
            bool allowExtensionlessDirectory)
        {
            fullDest = null;
            if (string.IsNullOrWhiteSpace(relAssetPath))
                return false;

            string normalized = relAssetPath.Replace('\\', '/');
            if (!normalized.StartsWith("Assets/", StringComparison.Ordinal) || normalized.Length <= "Assets/".Length)
                return false;

            // Reject drive prefixes, alternate data streams, wildcards, and C0/DEL/C1 control characters.
            for (int i = 0; i < normalized.Length; i++)
            {
                char ch = normalized[i];
                if (ch < 0x20 || (ch >= 0x7f && ch <= 0x9f) ||
                    ch == ':' || ch == '*' || ch == '?' || ch == '"' || ch == '<' || ch == '>' || ch == '|')
                {
                    return false;
                }
            }

            string[] segments = normalized.Split('/');
            if (segments.Length < 2 || !string.Equals(segments[0], "Assets", StringComparison.Ordinal))
                return false;

            for (int i = 0; i < segments.Length; i++)
            {
                string seg = segments[i];
                if (string.IsNullOrEmpty(seg) || seg == "." || seg == "..")
                    return false;
                // Reject Windows trailing dots/spaces and leading whitespace on every path segment before canonicalization.
                if (seg[0] == ' ' || seg[seg.Length - 1] == ' ' || seg[seg.Length - 1] == '.')
                    return false;
                string canonSeg = seg.TrimEnd('.', ' ');
                if (string.IsNullOrEmpty(canonSeg) || IsWindowsReservedName(canonSeg))
                    return false;
                if (i == 1 && string.Equals(canonSeg, "_TacSim", StringComparison.OrdinalIgnoreCase))
                    return false;
                if (i >= 1 && string.Equals(canonSeg, "Editor", StringComparison.OrdinalIgnoreCase))
                    return false;
            }

            string combined = Path.GetFullPath(Path.Combine(fullProjectRoot, normalized.Replace('/', Path.DirectorySeparatorChar)));
            // Use strict case-sensitive canonical prefix verification so Assets/../assets/... cannot escape on Linux/macOS.
            if (!combined.StartsWith(assetsPrefix, StringComparison.Ordinal))
                return false;

            // Re-validate every canonical segment and extension AFTER Path.GetFullPath (closes Windows canonicalization bypasses).
            string postRel = combined.Substring(assetsPrefix.Length);
            string[] postSegments = postRel.Split(new[] { Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar }, StringSplitOptions.None);
            if (postSegments.Length == 0)
                return false;
            for (int i = 0; i < postSegments.Length; i++)
            {
                string ps = postSegments[i];
                if (string.IsNullOrEmpty(ps) || ps == "." || ps == ".." ||
                    ps[0] == ' ' || ps[ps.Length - 1] == ' ' || ps[ps.Length - 1] == '.')
                {
                    return false;
                }
                string canonPs = ps.TrimEnd('.', ' ');
                if (string.IsNullOrEmpty(canonPs) || IsWindowsReservedName(canonPs))
                    return false;
                if (i == 0 && string.Equals(canonPs, "_TacSim", StringComparison.OrdinalIgnoreCase))
                    return false;
                if (string.Equals(canonPs, "Editor", StringComparison.OrdinalIgnoreCase))
                    return false;
            }

            string leaf = postSegments[postSegments.Length - 1];
            string ext = Path.GetExtension(leaf);
            if (!string.IsNullOrEmpty(ext))
            {
                if (BlockedExecutableExtensions.Contains(ext) || !AllowedPassiveAssetExtensions.Contains(ext))
                    return false;
            }
            else if (!allowExtensionlessDirectory)
            {
                return false;
            }

            fullDest = combined;
            return true;
        }

        private static bool TrySplitGuidEntry(string rawEntryPath, out string guid, out string leaf)
        {
            guid = null;
            leaf = null;
            if (string.IsNullOrWhiteSpace(rawEntryPath)) return false;

            string p = rawEntryPath.Replace('\\', '/').Trim();
            while (p.StartsWith("./", StringComparison.Ordinal))
                p = p.Substring(2);
            p = p.Trim('/');

            int slash = p.IndexOf('/');
            if (slash <= 0 || slash >= p.Length - 1)
                return false;

            guid = p.Substring(0, slash);
            leaf = p.Substring(slash + 1);
            return leaf.IndexOf('/') < 0;
        }

        private static void WalkTarEntries(
            Stream tarStream,
            long maxSingleEntryBytes,
            long maxTotalUncompressedBytes,
            Action<string, byte, long, Stream> onEntry)
        {
            byte[] header = new byte[512];
            string pendingLongName = null;
            long totalBytesConsumed = 0;
            int entryCount = 0;

            void AccountBytes(long delta)
            {
                if (delta < 0 || delta > maxTotalUncompressedBytes - totalBytesConsumed)
                    throw new InvalidDataException($"Archive exceeds maximum uncompressed limit ({maxTotalUncompressedBytes} bytes).");
                totalBytesConsumed += delta;
            }

            while (true)
            {
                int read = ReadUpToBlock(tarStream, header, 512);
                if (read == 0 || IsAllZero(header))
                    break;
                if (read < 512)
                    throw new InvalidDataException("Truncated tar header in .unitypackage.");

                AccountBytes(512);
                entryCount++;
                if (entryCount > MaxEntries)
                    throw new InvalidDataException($"Archive exceeds maximum entry count ({MaxEntries}).");

                ValidateTarChecksum(header);

                byte typeFlag = header[156];
                long size = ParseTarSize(header, 124, 12);
                if (size < 0 || size > maxSingleEntryBytes)
                    throw new InvalidDataException($"Tar entry size {size} exceeds permitted limit ({maxSingleEntryBytes} bytes).");

                string name = ParseNullTerminatedUtf8(header, 0, 100);
                string magic = ParseNullTerminatedUtf8(header, 257, 6);
                if (magic.StartsWith("ustar", StringComparison.OrdinalIgnoreCase))
                {
                    string prefix = ParseNullTerminatedUtf8(header, 345, 155);
                    if (!string.IsNullOrEmpty(prefix))
                        name = prefix.TrimEnd('/') + "/" + name.TrimStart('/');
                }

                if (!string.IsNullOrEmpty(pendingLongName))
                {
                    name = pendingLongName;
                    pendingLongName = null;
                }

                long pad = (512 - (size % 512)) % 512;
                AccountBytes(size);
                AccountBytes(pad);

                if (typeFlag == (byte)'L')
                {
                    if (size > MaxMetaHeaderBytes)
                        throw new InvalidDataException($"GNU longname header size {size} exceeds limit ({MaxMetaHeaderBytes}).");
                    byte[] nameBytes = ReadExact(tarStream, (int)size);
                    pendingLongName = ParseNullTerminatedUtf8(nameBytes, 0, nameBytes.Length);
                    SkipExact(tarStream, pad);
                    continue;
                }

                if (typeFlag == (byte)'x' || typeFlag == (byte)'g')
                {
                    if (size > MaxMetaHeaderBytes)
                        throw new InvalidDataException($"PAX header size {size} exceeds limit ({MaxMetaHeaderBytes}).");
                    byte[] paxBytes = ReadExact(tarStream, (int)size);
                    if (TryExtractPaxPath(paxBytes, out string paxPath))
                        pendingLongName = paxPath;
                    SkipExact(tarStream, pad);
                    continue;
                }

                if (typeFlag == (byte)'5')
                {
                    SkipExact(tarStream, size + pad);
                    continue;
                }

                // Strict tar typeFlag allowlist: accept only regular files ('0' or '\0') after metadata/dir headers.
                // Reject symlinks ('2'), hardlinks ('1'), devices ('3'/'4'), FIFOs ('6'), and unknown types.
                if (typeFlag != (byte)'0' && typeFlag != 0)
                    throw new InvalidDataException($"Unsupported tar entry typeFlag 0x{typeFlag:x2} in .unitypackage.");

                onEntry(name, typeFlag, size, tarStream);
                SkipExact(tarStream, pad);
            }
        }

        private static void ValidateTarChecksum(byte[] header)
        {
            long expected = ParseStrictOctal(header, 148, 8);
            long unsignedSum = 0;
            long signedSum = 0;
            for (int i = 0; i < 512; i++)
            {
                byte b = (i >= 148 && i < 156) ? (byte)' ' : header[i];
                unsignedSum += b;
                signedSum += (sbyte)b;
            }
            if (expected != unsignedSum && expected != signedSum)
                throw new InvalidDataException($"Invalid tar header checksum (expected {expected}, got {unsignedSum}).");
        }

        private static long ParseStrictOctal(byte[] buf, int offset, int length)
        {
            long octal = 0;
            bool sawDigit = false;
            bool sawTerminator = false;
            for (int i = 0; i < length; i++)
            {
                byte c = buf[offset + i];
                if (c == 0 || c == (byte)' ')
                {
                    if (sawDigit) sawTerminator = true;
                    continue;
                }
                if (sawTerminator || c < (byte)'0' || c > (byte)'7')
                    throw new InvalidDataException($"Invalid octal digit 0x{c:x2} in tar header.");
                if (octal > (long.MaxValue >> 3))
                    throw new InvalidDataException("Octal field overflow in tar header.");
                octal = (octal << 3) + (c - (byte)'0');
                sawDigit = true;
            }
            return octal;
        }

        private static bool TryExtractPaxPath(byte[] paxBytes, out string paxPath)
        {
            paxPath = null;
            string text = Encoding.UTF8.GetString(paxBytes);
            int idx = 0;
            while (idx < text.Length)
            {
                int nl = text.IndexOf('\n', idx);
                if (nl < 0) nl = text.Length;
                string line = text.Substring(idx, nl - idx);
                int sp = line.IndexOf(' ');
                if (sp > 0 && sp + 1 < line.Length)
                {
                    string kv = line.Substring(sp + 1);
                    if (kv.StartsWith("path=", StringComparison.Ordinal))
                    {
                        paxPath = kv.Substring("path=".Length);
                        return true;
                    }
                }
                idx = nl + 1;
            }
            return false;
        }

        private static long ParseTarSize(byte[] buf, int offset, int length)
        {
            if ((buf[offset] & 0x80) != 0)
            {
                // GNU base-256 positive integer only; reject negative two's complement or values > Int64.MaxValue.
                if ((buf[offset] & 0x40) != 0)
                    throw new InvalidDataException("Negative base-256 tar entry size is not supported.");
                if ((buf[offset] & 0x7F) != 0)
                    throw new InvalidDataException("Tar entry size exceeds 63-bit positive range.");

                // For standard 12-byte size field: bytes [offset+1..offset+3] must be 0, and byte [offset+4] sign bit must be 0.
                int lowStart = Math.Max(offset + 1, offset + length - 8);
                for (int i = offset + 1; i < lowStart; i++)
                {
                    if (buf[i] != 0)
                        throw new InvalidDataException("Tar entry size exceeds 63-bit positive range.");
                }
                if (length >= 9 && (buf[lowStart] & 0x80) != 0)
                    throw new InvalidDataException("Tar entry size exceeds Int64.MaxValue.");

                ulong acc = 0;
                for (int i = lowStart; i < offset + length; i++)
                    acc = (acc << 8) | buf[i];
                if (acc > (ulong)long.MaxValue)
                    throw new InvalidDataException("Tar entry size exceeds Int64.MaxValue.");
                return (long)acc;
            }

            return ParseStrictOctal(buf, offset, length);
        }

        private static string ParseNullTerminatedUtf8(byte[] buf, int offset, int maxLen)
        {
            int end = offset;
            int limit = offset + maxLen;
            while (end < limit && buf[end] != 0)
                end++;
            return end > offset ? Encoding.UTF8.GetString(buf, offset, end - offset) : string.Empty;
        }

        private static bool IsAllZero(byte[] buf)
        {
            for (int i = 0; i < buf.Length; i++)
                if (buf[i] != 0) return false;
            return true;
        }

        private static int ReadUpToBlock(Stream s, byte[] buf, int count)
        {
            int total = 0;
            while (total < count)
            {
                int n = s.Read(buf, total, count - total);
                if (n <= 0) break;
                total += n;
            }
            return total;
        }

        private static byte[] ReadExact(Stream s, int count)
        {
            byte[] buf = new byte[count];
            int r = ReadUpToBlock(s, buf, count);
            if (r < count) throw new EndOfStreamException("Unexpected EOF while reading .unitypackage entry.");
            return buf;
        }

        private static void CopyExact(Stream src, Stream dst, long bytesToCopy, byte[] scratch)
        {
            long remaining = bytesToCopy;
            while (remaining > 0)
            {
                int chunk = (int)Math.Min(remaining, scratch.Length);
                int n = src.Read(scratch, 0, chunk);
                if (n <= 0) throw new EndOfStreamException("Unexpected EOF while extracting .unitypackage asset.");
                dst.Write(scratch, 0, n);
                remaining -= n;
            }
        }

        private static void SkipExact(Stream s, long bytesToSkip)
        {
            if (bytesToSkip <= 0) return;
            if (s.CanSeek)
            {
                s.Seek(bytesToSkip, SeekOrigin.Current);
                return;
            }
            byte[] discard = new byte[16384];
            long remaining = bytesToSkip;
            while (remaining > 0)
            {
                int chunk = (int)Math.Min(remaining, discard.Length);
                int n = s.Read(discard, 0, chunk);
                if (n <= 0) throw new EndOfStreamException("Unexpected EOF while skipping .unitypackage padding.");
                remaining -= n;
            }
        }
    }
}
