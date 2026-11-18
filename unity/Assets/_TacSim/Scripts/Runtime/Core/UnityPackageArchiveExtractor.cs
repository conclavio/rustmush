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
            if (seekableGzipTarStream == null) throw new ArgumentNullException(nameof(seekableGzipTarStream));
            if (string.IsNullOrWhiteSpace(projectRootDir)) throw new ArgumentNullException(nameof(projectRootDir));

            string fullProjectRoot = Path.GetFullPath(projectRootDir);
            string fullAssetsRoot = Path.GetFullPath(Path.Combine(fullProjectRoot, "Assets"));
            string assetsPrefix = fullAssetsRoot.TrimEnd(Path.DirectorySeparatorChar, Path.AltDirectorySeparatorChar) + Path.DirectorySeparatorChar;

            // Pass 1: Collect `<guid>/pathname` mappings (handles archives where `<guid>/asset` precedes `<guid>/pathname`).
            seekableGzipTarStream.Position = 0;
            var guidToRelPath = new Dictionary<string, string>(StringComparer.OrdinalIgnoreCase);
            var folderGuidsWithPayload = new HashSet<string>(StringComparer.OrdinalIgnoreCase);

            using (var gz1 = new GZipStream(seekableGzipTarStream, CompressionMode.Decompress, leaveOpen: true))
            {
                WalkTarEntries(gz1, (entryPath, typeFlag, size, dataStream) =>
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
                        string cleaned = raw.Trim().Replace('\\', '/');
                        if (cleaned.StartsWith("Assets/", StringComparison.Ordinal) && cleaned.Length > "Assets/".Length)
                            guidToRelPath[guid] = cleaned;
                    }
                    else
                    {
                        if (string.Equals(leaf, "asset", StringComparison.OrdinalIgnoreCase))
                            folderGuidsWithPayload.Add(guid);
                        SkipExact(dataStream, size);
                    }
                });
            }

            // First ensure all folder entries (GUIDs that have a pathname & meta, but no `asset` file payload) exist as directories.
            foreach (var kv in guidToRelPath)
            {
                if (!folderGuidsWithPayload.Contains(kv.Key) &&
                    TryResolveSafeAssetDestination(fullProjectRoot, assetsPrefix, kv.Value, out string folderFull))
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
                WalkTarEntries(gz2, (entryPath, typeFlag, size, dataStream) =>
                {
                    if (!TrySplitGuidEntry(entryPath, out string guid, out string leaf) ||
                        !guidToRelPath.TryGetValue(guid, out string relAssetPath) ||
                        !TryResolveSafeAssetDestination(fullProjectRoot, assetsPrefix, relAssetPath, out string destAssetFull))
                    {
                        SkipExact(dataStream, size);
                        return;
                    }

                    if (string.Equals(leaf, "asset", StringComparison.OrdinalIgnoreCase))
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
                    else if (string.Equals(leaf, "asset.meta", StringComparison.OrdinalIgnoreCase))
                    {
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
                    else
                    {
                        SkipExact(dataStream, size);
                    }
                });
            }

            return filesWritten;
        }

        private static bool TryResolveSafeAssetDestination(string fullProjectRoot, string assetsPrefix, string relAssetPath, out string fullDest)
        {
            fullDest = null;
            if (string.IsNullOrWhiteSpace(relAssetPath) || !relAssetPath.StartsWith("Assets/", StringComparison.Ordinal))
                return false;

            string combined = Path.GetFullPath(Path.Combine(fullProjectRoot, relAssetPath.Replace('/', Path.DirectorySeparatorChar)));
            if (!combined.StartsWith(assetsPrefix, StringComparison.OrdinalIgnoreCase))
                return false;

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

        private static void WalkTarEntries(Stream tarStream, Action<string, byte, long, Stream> onEntry)
        {
            byte[] header = new byte[512];
            string pendingLongName = null;

            while (true)
            {
                int read = ReadUpToBlock(tarStream, header, 512);
                if (read == 0 || IsAllZero(header))
                    break;
                if (read < 512)
                    throw new InvalidDataException("Truncated tar header in .unitypackage.");

                byte typeFlag = header[156];
                long size = ParseTarSize(header, 124, 12);

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

                if (typeFlag == (byte)'L')
                {
                    byte[] nameBytes = ReadExact(tarStream, (int)size);
                    pendingLongName = ParseNullTerminatedUtf8(nameBytes, 0, nameBytes.Length);
                    SkipExact(tarStream, pad);
                    continue;
                }

                if (typeFlag == (byte)'x' || typeFlag == (byte)'g')
                {
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

                onEntry(name, typeFlag, size, tarStream);
                SkipExact(tarStream, pad);
            }
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
                long val = 0;
                for (int i = 1; i < length; i++)
                    val = (val << 8) | buf[offset + i];
                return val;
            }

            long octal = 0;
            for (int i = 0; i < length; i++)
            {
                byte c = buf[offset + i];
                if (c == 0 || c == (byte)' ')
                {
                    if (octal > 0) break;
                    continue;
                }
                if (c >= (byte)'0' && c <= (byte)'7')
                    octal = (octal << 3) + (c - (byte)'0');
            }
            return octal;
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
