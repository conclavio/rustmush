// Headless .NET CI compile & math/data surface for TacSim Unity 6 scripts (`unity/CI~/TacSim.CI.csproj`).
// Unity's AssetDatabase ignores `*~` directories, so this file is never imported inside the Unity Editor.
using System;
using System.Collections;
using System.Collections.Generic;

namespace UnityEngine
{
    public sealed class SerializeField : Attribute { }
    public sealed class HeaderAttribute : Attribute { public HeaderAttribute(string _) { } }
    public sealed class TooltipAttribute : Attribute { public TooltipAttribute(string _) { } }
    public sealed class RangeAttribute : Attribute { public RangeAttribute(float min, float max) { } }
    public sealed class RequireComponent : Attribute
    {
        public RequireComponent(Type a) { }
        public RequireComponent(Type a, Type b) { }
        public RequireComponent(Type a, Type b, Type c) { }
    }
    public sealed class CreateAssetMenuAttribute : Attribute
    {
        public string fileName;
        public string menuName;
        public int order;
    }
    public sealed class DisallowMultipleComponent : Attribute { }
    public sealed class ExecuteAlways : Attribute { }
    public sealed class HideInInspector : Attribute { }
    public enum RuntimeInitializeLoadType { AfterSceneLoad, BeforeSceneLoad, SubsystemRegistration }
    public sealed class RuntimeInitializeOnLoadMethodAttribute : Attribute
    {
        public RuntimeInitializeOnLoadMethodAttribute() { }
        public RuntimeInitializeOnLoadMethodAttribute(RuntimeInitializeLoadType _) { }
    }

    public static class Mathf
    {
        public const float PI = (float)Math.PI;
        public const float Deg2Rad = PI / 180f;
        public const float Rad2Deg = 180f / PI;
        public const float Epsilon = 1.192092896e-07F;
        public const float Infinity = float.PositiveInfinity;

        public static float Abs(float f) => Math.Abs(f);
        public static int Abs(int v) => Math.Abs(v);
        public static float Min(float a, float b) => a < b ? a : b;
        public static int Min(int a, int b) => a < b ? a : b;
        public static float Max(float a, float b) => a > b ? a : b;
        public static int Max(int a, int b) => a > b ? a : b;
        public static float Clamp(float v, float min, float max) => v < min ? min : (v > max ? max : v);
        public static int Clamp(int v, int min, int max) => v < min ? min : (v > max ? max : v);
        public static float Clamp01(float v) => Clamp(v, 0f, 1f);
        public static float Lerp(float a, float b, float t) => a + (b - a) * Clamp01(t);
        public static float LerpUnclamped(float a, float b, float t) => a + (b - a) * t;
        public static float InverseLerp(float a, float b, float v) => Math.Abs(b - a) > 1e-6f ? Clamp01((v - a) / (b - a)) : 0f;
        public static float Sin(float f) => (float)Math.Sin(f);
        public static float Cos(float f) => (float)Math.Cos(f);
        public static float Tan(float f) => (float)Math.Tan(f);
        public static float Asin(float f) => (float)Math.Asin(f);
        public static float Acos(float f) => (float)Math.Acos(f);
        public static float Atan2(float y, float x) => (float)Math.Atan2(y, x);
        public static float Sqrt(float f) => (float)Math.Sqrt(f);
        public static float Pow(float f, float p) => (float)Math.Pow(f, p);
        public static float Log(float f) => (float)Math.Log(f);
        public static float Exp(float p) => (float)Math.Exp(p);
        public static float Floor(float f) => (float)Math.Floor(f);
        public static float Ceil(float f) => (float)Math.Ceiling(f);
        public static float Round(float f) => (float)Math.Round(f);
        public static int FloorToInt(float f) => (int)Math.Floor(f);
        public static int CeilToInt(float f) => (int)Math.Ceiling(f);
        public static int RoundToInt(float f) => (int)Math.Round(f);
        public static float Sign(float f) => f >= 0f ? 1f : -1f;
        public static float Repeat(float t, float length) => Clamp(t - Floor(t / length) * length, 0f, length);
        public static float PingPong(float t, float length) { t = Repeat(t, length * 2f); return length - Abs(t - length); }
        public static float DeltaAngle(float current, float target)
        {
            float num = Repeat(target - current, 360f);
            if (num > 180f) num -= 360f;
            return num;
        }
        public static float LerpAngle(float a, float b, float t) => a + DeltaAngle(a, b) * Clamp01(t);
        public static float MoveTowards(float current, float target, float maxDelta) =>
            Abs(target - current) <= maxDelta ? target : current + Sign(target - current) * maxDelta;
        public static float MoveTowardsAngle(float current, float target, float maxDelta)
        {
            float d = DeltaAngle(current, target);
            if (-maxDelta < d && d < maxDelta) return target;
            return MoveTowards(current, current + d, maxDelta);
        }
        public static float SmoothStep(float from, float to, float t)
        {
            t = Clamp01(t);
            t = -2f * t * t * t + 3f * t * t;
            return to * t + from * (1f - t);
        }
        public static bool Approximately(float a, float b) => Abs(b - a) < Max(1e-6f * Max(Abs(a), Abs(b)), Epsilon * 8f);
        public static float PerlinNoise(float x, float y) => 0.5f + 0.5f * Sin(x * 1.7f + Cos(y * 2.3f));
    }

    public struct Vector2 : IEquatable<Vector2>
    {
        public float x, y;
        public Vector2(float x, float y) { this.x = x; this.y = y; }
        public static Vector2 zero => new Vector2(0f, 0f);
        public static Vector2 one => new Vector2(1f, 1f);
        public static Vector2 up => new Vector2(0f, 1f);
        public static Vector2 down => new Vector2(0f, -1f);
        public static Vector2 left => new Vector2(-1f, 0f);
        public static Vector2 right => new Vector2(1f, 0f);
        public float magnitude => Mathf.Sqrt(x * x + y * y);
        public float sqrMagnitude => x * x + y * y;
        public Vector2 normalized { get { float m = magnitude; return m > 1e-5f ? this / m : zero; } }
        public void Normalize() { this = normalized; }
        public static float Distance(Vector2 a, Vector2 b) => (a - b).magnitude;
        public static float Dot(Vector2 a, Vector2 b) => a.x * b.x + a.y * b.y;
        public static Vector2 Lerp(Vector2 a, Vector2 b, float t) => a + (b - a) * Mathf.Clamp01(t);
        public static Vector2 operator +(Vector2 a, Vector2 b) => new Vector2(a.x + b.x, a.y + b.y);
        public static Vector2 operator -(Vector2 a, Vector2 b) => new Vector2(a.x - b.x, a.y - b.y);
        public static Vector2 operator -(Vector2 a) => new Vector2(-a.x, -a.y);
        public static Vector2 operator *(Vector2 a, float d) => new Vector2(a.x * d, a.y * d);
        public static Vector2 operator *(float d, Vector2 a) => new Vector2(a.x * d, a.y * d);
        public static Vector2 operator /(Vector2 a, float d) => new Vector2(a.x / d, a.y / d);
        public bool Equals(Vector2 other) => x == other.x && y == other.y;
        public override bool Equals(object obj) => obj is Vector2 o && Equals(o);
        public override int GetHashCode() => HashCode.Combine(x, y);
    }

    public struct Vector3 : IEquatable<Vector3>
    {
        public float x, y, z;
        public Vector3(float x, float y, float z) { this.x = x; this.y = y; this.z = z; }
        public Vector3(float x, float y) { this.x = x; this.y = y; this.z = 0f; }
        public static Vector3 zero => new Vector3(0f, 0f, 0f);
        public static Vector3 one => new Vector3(1f, 1f, 1f);
        public static Vector3 up => new Vector3(0f, 1f, 0f);
        public static Vector3 down => new Vector3(0f, -1f, 0f);
        public static Vector3 left => new Vector3(-1f, 0f, 0f);
        public static Vector3 right => new Vector3(1f, 0f, 0f);
        public static Vector3 forward => new Vector3(0f, 0f, 1f);
        public static Vector3 back => new Vector3(0f, 0f, -1f);
        public float magnitude => Mathf.Sqrt(x * x + y * y + z * z);
        public float sqrMagnitude => x * x + y * y + z * z;
        public Vector3 normalized { get { float m = magnitude; return m > 1e-5f ? this / m : zero; } }
        public static float Distance(Vector3 a, Vector3 b) => (a - b).magnitude;
        public static float Dot(Vector3 a, Vector3 b) => a.x * b.x + a.y * b.y + a.z * b.z;
        public static float Angle(Vector3 from, Vector3 to)
        {
            float denom = MathF.Sqrt(from.sqrMagnitude * to.sqrMagnitude);
            if (denom < 1e-15f) return 0f;
            float dot = Mathf.Clamp(Dot(from, to) / denom, -1f, 1f);
            return MathF.Acos(dot) * Mathf.Rad2Deg;
        }
        public static Vector3 Cross(Vector3 a, Vector3 b) =>
            new Vector3(a.y * b.z - a.z * b.y, a.z * b.x - a.x * b.z, a.x * b.y - a.y * b.x);
        public static Vector3 ProjectOnPlane(Vector3 vector, Vector3 planeNormal)
        {
            float sqrMag = Dot(planeNormal, planeNormal);
            return sqrMag < Mathf.Epsilon ? vector : vector - planeNormal * (Dot(vector, planeNormal) / sqrMag);
        }
        public static Vector3 Lerp(Vector3 a, Vector3 b, float t) => a + (b - a) * Mathf.Clamp01(t);
        public static Vector3 MoveTowards(Vector3 current, Vector3 target, float maxDistanceDelta)
        {
            Vector3 toVec = target - current;
            float sqDist = toVec.sqrMagnitude;
            if (sqDist == 0f || (maxDistanceDelta >= 0f && sqDist <= maxDistanceDelta * maxDistanceDelta))
                return target;
            float dist = Mathf.Sqrt(sqDist);
            return current + toVec / dist * maxDistanceDelta;
        }
        public static Vector3 Min(Vector3 a, Vector3 b) => new Vector3(Mathf.Min(a.x, b.x), Mathf.Min(a.y, b.y), Mathf.Min(a.z, b.z));
        public static Vector3 Max(Vector3 a, Vector3 b) => new Vector3(Mathf.Max(a.x, b.x), Mathf.Max(a.y, b.y), Mathf.Max(a.z, b.z));
        public static Vector3 Scale(Vector3 a, Vector3 b) => new Vector3(a.x * b.x, a.y * b.y, a.z * b.z);
        public static Vector3 operator +(Vector3 a, Vector3 b) => new Vector3(a.x + b.x, a.y + b.y, a.z + b.z);
        public static Vector3 operator -(Vector3 a, Vector3 b) => new Vector3(a.x - b.x, a.y - b.y, a.z - b.z);
        public static Vector3 operator -(Vector3 a) => new Vector3(-a.x, -a.y, -a.z);
        public static Vector3 operator *(Vector3 a, float d) => new Vector3(a.x * d, a.y * d, a.z * d);
        public static Vector3 operator *(float d, Vector3 a) => new Vector3(a.x * d, a.y * d, a.z * d);
        public static Vector3 operator /(Vector3 a, float d) => new Vector3(a.x / d, a.y / d, a.z / d);
        public static bool operator ==(Vector3 a, Vector3 b) => (a - b).sqrMagnitude < 1e-10f;
        public static bool operator !=(Vector3 a, Vector3 b) => !(a == b);
        public bool Equals(Vector3 other) => this == other;
        public override bool Equals(object obj) => obj is Vector3 o && Equals(o);
        public override int GetHashCode() => HashCode.Combine(x, y, z);
        public override string ToString() => $"({x:0.###}, {y:0.###}, {z:0.###})";
    }

    public struct Vector4
    {
        public float x, y, z, w;
        public Vector4(float x, float y, float z, float w) { this.x = x; this.y = y; this.z = z; this.w = w; }
        public static implicit operator Vector4(Color c) => new Vector4(c.r, c.g, c.b, c.a);
    }

    public struct Quaternion
    {
        public float x, y, z, w;
        public Quaternion(float x, float y, float z, float w) { this.x = x; this.y = y; this.z = z; this.w = w; }
        public static Quaternion identity => new Quaternion(0f, 0f, 0f, 1f);
        public Vector3 eulerAngles { get => Vector3.zero; set { } }
        public static Quaternion Euler(float x, float y, float z)
        {
            float cx = MathF.Cos(x * 0.5f * Mathf.Deg2Rad), sx = MathF.Sin(x * 0.5f * Mathf.Deg2Rad);
            float cy = MathF.Cos(y * 0.5f * Mathf.Deg2Rad), sy = MathF.Sin(y * 0.5f * Mathf.Deg2Rad);
            float cz = MathF.Cos(z * 0.5f * Mathf.Deg2Rad), sz = MathF.Sin(z * 0.5f * Mathf.Deg2Rad);
            return new Quaternion(
                sx * cy * cz + cx * sy * sz,
                cx * sy * cz - sx * cy * sz,
                cx * cy * sz - sx * sy * cz,
                cx * cy * cz + sx * sy * sz);
        }
        public static Quaternion Euler(Vector3 euler) => Euler(euler.x, euler.y, euler.z);
        public static float Dot(Quaternion a, Quaternion b) => a.x * b.x + a.y * b.y + a.z * b.z + a.w * b.w;
        public static float Angle(Quaternion a, Quaternion b)
        {
            float dot = MathF.Min(MathF.Abs(Dot(a, b)), 1f);
            return dot > 0.999999f ? 0f : MathF.Acos(dot) * 2f * Mathf.Rad2Deg;
        }
        public static Quaternion LookRotation(Vector3 forward) => identity;
        public static Quaternion LookRotation(Vector3 forward, Vector3 upwards) => identity;
        public static Quaternion FromToRotation(Vector3 fromDirection, Vector3 toDirection) => identity;
        public static Quaternion Slerp(Quaternion a, Quaternion b, float t) => b;
        public static Quaternion Lerp(Quaternion a, Quaternion b, float t) => b;
        public static Quaternion AngleAxis(float angle, Vector3 axis) => identity;
        public static Quaternion Inverse(Quaternion rotation) => identity;
        public static Quaternion operator *(Quaternion lhs, Quaternion rhs) => identity;
        public static Vector3 operator *(Quaternion rotation, Vector3 point) => point;
    }

    public struct Color
    {
        public float r, g, b, a;
        public Color(float r, float g, float b, float a = 1f) { this.r = r; this.g = g; this.b = b; this.a = a; }
        public static Color white => new Color(1f, 1f, 1f, 1f);
        public static Color black => new Color(0f, 0f, 0f, 1f);
        public static Color clear => new Color(0f, 0f, 0f, 0f);
        public static Color gray => new Color(0.5f, 0.5f, 0.5f, 1f);
        public static Color grey => gray;
        public static Color red => new Color(1f, 0f, 0f, 1f);
        public static Color green => new Color(0f, 1f, 0f, 1f);
        public static Color blue => new Color(0f, 0f, 1f, 1f);
        public static Color cyan => new Color(0f, 1f, 1f, 1f);
        public static Color yellow => new Color(1f, 0.92f, 0.016f, 1f);
        public float maxColorComponent => Mathf.Max(r, Mathf.Max(g, b));
        public static Color HSVToRGB(float H, float S, float V) => new Color(V, V * (1f - S * 0.5f), V * (1f - S), 1f);
        public static Color Lerp(Color a, Color b, float t) =>
            new Color(Mathf.Lerp(a.r, b.r, t), Mathf.Lerp(a.g, b.g, t), Mathf.Lerp(a.b, b.b, t), Mathf.Lerp(a.a, b.a, t));
        public static Color operator *(Color a, float b) => new Color(a.r * b, a.g * b, a.b * b, a.a * b);
        public static Color operator *(float b, Color a) => a * b;
        public static Color operator +(Color a, Color b) => new Color(a.r + b.r, a.g + b.g, a.b + b.b, a.a + b.a);
    }

    public struct Color32
    {
        public byte r, g, b, a;
        public Color32(byte r, byte g, byte b, byte a) { this.r = r; this.g = g; this.b = b; this.a = a; }
        public static implicit operator Color(Color32 c) => new Color(c.r / 255f, c.g / 255f, c.b / 255f, c.a / 255f);
        public static implicit operator Color32(Color c) =>
            new Color32((byte)Mathf.Clamp(Mathf.RoundToInt(c.r * 255f), 0, 255),
                        (byte)Mathf.Clamp(Mathf.RoundToInt(c.g * 255f), 0, 255),
                        (byte)Mathf.Clamp(Mathf.RoundToInt(c.b * 255f), 0, 255),
                        (byte)Mathf.Clamp(Mathf.RoundToInt(c.a * 255f), 0, 255));
    }

    public struct Rect
    {
        public float x, y, width, height;
        public Rect(float x, float y, float width, float height) { this.x = x; this.y = y; this.width = width; this.height = height; }
        public float xMin { get => x; set { float xmax = xMax; x = value; width = xmax - x; } }
        public float yMin { get => y; set { float ymax = yMax; y = value; height = ymax - y; } }
        public float xMax { get => x + width; set => width = value - x; }
        public float yMax { get => y + height; set => height = value - y; }
    }

    public struct Bounds
    {
        private Vector3 _center;
        private Vector3 _extents;
        public Bounds(Vector3 center, Vector3 size) { _center = center; _extents = size * 0.5f; }
        public Vector3 center { get => _center; set => _center = value; }
        public Vector3 extents { get => _extents; set => _extents = value; }
        public Vector3 size { get => _extents * 2f; set => _extents = value * 0.5f; }
        public Vector3 min => _center - _extents;
        public Vector3 max => _center + _extents;
        public void Encapsulate(Vector3 point)
        {
            Vector3 mn = Vector3.Min(min, point);
            Vector3 mx = Vector3.Max(max, point);
            _center = (mn + mx) * 0.5f;
            _extents = (mx - mn) * 0.5f;
        }
        public void Encapsulate(Bounds b) { Encapsulate(b.min); Encapsulate(b.max); }
    }

    public struct Ray
    {
        public Vector3 origin;
        public Vector3 direction;
        public Ray(Vector3 o, Vector3 d) { origin = o; direction = d.normalized; }
    }

    public struct RaycastHit
    {
        public Vector3 point;
        public Vector3 normal;
        public float distance;
        public Collider collider;
        public Transform transform => collider?.transform;
    }

    public enum QueryTriggerInteraction { UseGlobal = 0, Ignore = 1, Collide = 2 }

    public static class Physics
    {
        public static bool Raycast(Ray ray, out RaycastHit hitInfo, float maxDistance = float.PositiveInfinity, int layerMask = -1, QueryTriggerInteraction queryTriggerInteraction = QueryTriggerInteraction.UseGlobal)
        {
            hitInfo = default;
            return false;
        }
        public static bool Raycast(Vector3 origin, Vector3 direction, out RaycastHit hitInfo, float maxDistance = float.PositiveInfinity, int layerMask = -1, QueryTriggerInteraction queryTriggerInteraction = QueryTriggerInteraction.UseGlobal)
        {
            hitInfo = default;
            return false;
        }
        public static bool SphereCast(Vector3 origin, float radius, Vector3 direction, out RaycastHit hitInfo, float maxDistance = float.PositiveInfinity, int layerMask = -1, QueryTriggerInteraction queryTriggerInteraction = QueryTriggerInteraction.UseGlobal)
        {
            hitInfo = default;
            return false;
        }
        public static Collider[] OverlapSphere(Vector3 position, float radius, int layerMask = -1) => Array.Empty<Collider>();
        public static int OverlapSphereNonAlloc(Vector3 position, float radius, Collider[] results, int layerMask = -1) => 0;
    }

    public static class PlayerPrefs
    {
        public static string GetString(string key, string defaultValue = "") => defaultValue;
        public static void SetString(string key, string value) { }
        public static int GetInt(string key, int defaultValue = 0) => defaultValue;
        public static void SetInt(string key, int value) { }
        public static void Save() { }
    }

    public enum Space { World, Self }
    public enum PrimitiveType { Sphere, Capsule, Cylinder, Cube, Plane, Quad }
    public enum LightType { Spot, Directional, Point, Area }
    public enum LightShadows { None, Hard, Soft }
    public enum CameraClearFlags { Skybox = 1, Color = 2, SolidColor = 2, Depth = 3, Nothing = 4 }
    public enum CursorLockMode { None, Locked, Confined }
    public enum FontStyle { Normal, Bold, Italic, BoldAndItalic }
    public enum TextAnchor { UpperLeft, UpperCenter, UpperRight, MiddleLeft, MiddleCenter, MiddleRight, LowerLeft, LowerCenter, LowerRight }
    public enum HorizontalWrapMode { Wrap, Overflow }
    public enum VerticalWrapMode { Truncate, Overflow }
    public enum AudioRolloffMode { Logarithmic, Linear, Custom }
    public enum CollisionFlags { None = 0, Sides = 1, Above = 2, Below = 4 }
    public enum SendMessageOptions { RequireReceiver, DontRequireReceiver }
    public enum HideFlags { None = 0, HideAndDontSave = 61 }

    public enum KeyCode
    {
        None = 0, Backspace = 8, Tab = 9, Return = 13, Escape = 27, Space = 32,
        Alpha0 = 48, Alpha1 = 49, Alpha2 = 50, Alpha3 = 51, Alpha4 = 52, Alpha5 = 53, Alpha6 = 54, Alpha7 = 55, Alpha8 = 56, Alpha9 = 57,
        A = 97, B = 98, C = 99, D = 100, E = 101, F = 102, G = 103, H = 104, I = 105, J = 106, K = 107, L = 108, M = 109,
        N = 110, O = 111, P = 112, Q = 113, R = 114, S = 115, T = 116, U = 117, V = 118, W = 119, X = 120, Y = 121, Z = 122,
        KeypadEnter = 271, UpArrow = 273, DownArrow = 274, RightArrow = 275, LeftArrow = 276,
        F1 = 282, F2 = 283, F3 = 284, F4 = 285, F5 = 286, F6 = 287, F7 = 288, F8 = 289, F9 = 290, F10 = 291, F11 = 292, F12 = 293,
        RightShift = 303, LeftShift = 304, RightControl = 305, LeftControl = 306
    }

    public static class Input
    {
        public static Vector3 mousePosition => Vector3.zero;
        public static Vector2 mouseScrollDelta => Vector2.zero;
        public static bool GetKey(KeyCode key) => false;
        public static bool GetKeyDown(KeyCode key) => false;
        public static bool GetKeyUp(KeyCode key) => false;
        public static bool GetMouseButton(int button) => false;
        public static bool GetMouseButtonDown(int button) => false;
        public static bool GetMouseButtonUp(int button) => false;
        public static float GetAxis(string axisName) => 0f;
        public static float GetAxisRaw(string axisName) => 0f;
    }

    public static class Cursor
    {
        public static CursorLockMode lockState { get; set; }
        public static bool visible { get; set; } = true;
    }

    public static class Screen
    {
        public static int width => 1920;
        public static int height => 1080;
    }

    public static class Time
    {
        public static float time { get; set; }
        public static float unscaledTime { get; set; }
        public static float realtimeSinceStartup { get; set; }
        public static float deltaTime { get; set; } = 0.016f;
        public static float unscaledDeltaTime { get; set; } = 0.016f;
        public static float fixedDeltaTime { get; set; } = 0.02f;
    }

    public static class Application
    {
        public static bool isPlaying => false;
        public static string dataPath => string.Empty;
        public static string persistentDataPath => string.Empty;
        public static int targetFrameRate { get; set; }
        public static void Quit() { }
    }

    public static class Debug
    {
        public static void Log(object message) { }
        public static void LogWarning(object message) { }
        public static void LogError(object message) { }
        public static void LogException(Exception exception) { }
    }

    public static class QualitySettings
    {
        public static int vSyncCount { get; set; }
        public static Rendering.RenderPipelineAsset renderPipeline { get; set; }
    }

    public static class RenderSettings
    {
        public static Material skybox { get; set; }
        public static Rendering.AmbientMode ambientMode { get; set; }
        public static Color ambientLight { get; set; }
        public static Color ambientSkyColor { get; set; }
        public static Color ambientEquatorColor { get; set; }
        public static Color ambientGroundColor { get; set; }
        public static float ambientIntensity { get; set; }
        public static bool fog { get; set; }
        public static Color fogColor { get; set; }
        public static float fogDensity { get; set; }
    }

    public class Object
    {
        public virtual string name { get; set; } = string.Empty;
        public HideFlags hideFlags { get; set; }
        public static void Destroy(Object obj, float t = 0f)
        {
            if (!Application.isPlaying && obj != null)
                throw new InvalidOperationException("Destroy may not be called from edit mode! Use DestroyImmediate instead.");
        }
        public static void DestroyImmediate(Object obj, bool allowDestroyingAssets = false)
        {
            if (obj is GameObject go)
            {
                var cams = go.GetComponentsInChildren<Camera>(includeInactive: true);
                for (int i = 0; i < cams.Length; i++)
                    Camera.Unregister(cams[i]);
                if (go.transform != null)
                    go.transform.SetParent(null);
            }
            else if (obj is Component comp && comp.gameObject != null)
            {
                if (comp is Camera cam)
                    Camera.Unregister(cam);
                comp.gameObject.RemoveComponentInternal(comp);
            }
        }
        public static void DontDestroyOnLoad(Object target) { }
        public static T Instantiate<T>(T original) where T : Object => original;
        public static T Instantiate<T>(T original, Transform parent, bool instantiateInWorldSpace = false) where T : Object => original;
        public static T Instantiate<T>(T original, Vector3 position, Quaternion rotation) where T : Object => original;
        public static T Instantiate<T>(T original, Vector3 position, Quaternion rotation, Transform parent) where T : Object => original;
        public static GameObject Instantiate(GameObject original) => new GameObject(original?.name ?? "Clone");
        public static GameObject Instantiate(GameObject original, Transform parent, bool worldPositionStays = false)
        {
            var go = new GameObject(original?.name ?? "Clone");
            go.transform.SetParent(parent, worldPositionStays);
            return go;
        }
        public static GameObject Instantiate(GameObject original, Vector3 position, Quaternion rotation)
        {
            var go = new GameObject(original?.name ?? "Clone");
            go.transform.position = position;
            go.transform.rotation = rotation;
            return go;
        }
        public static GameObject Instantiate(GameObject original, Vector3 position, Quaternion rotation, Transform parent)
        {
            var go = Instantiate(original, position, rotation);
            go.transform.SetParent(parent, true);
            return go;
        }
        public static T FindFirstObjectByType<T>() where T : Object => null;
        public static T FindAnyObjectByType<T>() where T : Object => null;
        public static T[] FindObjectsByType<T>(FindObjectsSortMode _) where T : Object => Array.Empty<T>();
    }

    public enum FindObjectsSortMode { None, InstanceID }

    public class ScriptableObject : Object
    {
        public static T CreateInstance<T>() where T : ScriptableObject, new() => new T();
    }

    public class TextAsset : Object
    {
        public string text { get; set; } = string.Empty;
        public byte[] bytes { get; set; } = Array.Empty<byte>();
    }

    public static class Resources
    {
        public static T Load<T>(string path) where T : Object => null;
        public static ResourceRequest LoadAsync<T>(string path) where T : Object => new ResourceRequest();
        public static T GetBuiltinResource<T>(string path) where T : Object
        {
            if (typeof(T) == typeof(Font)) return new Font() as T;
            return null;
        }
    }

    public class ResourceRequest
    {
        public bool isDone => true;
        public Object asset => null;
    }

    public class Component : Object
    {
        public GameObject gameObject { get; internal set; }
        public Transform transform => gameObject?.transform;
        public override string name
        {
            get => gameObject != null ? gameObject.name : base.name;
            set { if (gameObject != null) gameObject.name = value; else base.name = value; }
        }

        public T GetComponent<T>() => gameObject != null ? gameObject.GetComponent<T>() : default;
        public T GetComponentInChildren<T>(bool includeInactive = false) => gameObject != null ? gameObject.GetComponentInChildren<T>(includeInactive) : default;
        public T GetComponentInParent<T>() => GetComponent<T>();
        public T[] GetComponentsInChildren<T>(bool includeInactive = false) => gameObject != null ? gameObject.GetComponentsInChildren<T>(includeInactive) : Array.Empty<T>();
        public T[] GetComponents<T>() => gameObject != null ? gameObject.GetComponents<T>() : Array.Empty<T>();
    }

    public class Behaviour : Component
    {
        public bool enabled { get; set; } = true;
        public bool isActiveAndEnabled => enabled && (gameObject == null || gameObject.activeInHierarchy);
    }

    public class MonoBehaviour : Behaviour
    {
        public Coroutine StartCoroutine(IEnumerator routine) => new Coroutine();
        public void StopCoroutine(Coroutine routine) { }
        public void StopAllCoroutines() { }
    }

    public sealed class Coroutine { }

    public class Transform : Component, IEnumerable<Transform>
    {
        private readonly List<Transform> _children = new List<Transform>();
        public Transform parent { get; private set; }
        public Vector3 position { get; set; }
        public Vector3 localPosition { get; set; }
        public Quaternion rotation { get; set; } = Quaternion.identity;
        public Quaternion localRotation { get; set; } = Quaternion.identity;
        public Vector3 eulerAngles { get => Vector3.zero; set { } }
        public Vector3 localEulerAngles { get => Vector3.zero; set { } }
        public Vector3 localScale { get; set; } = Vector3.one;
        public Vector3 lossyScale => localScale;
        public Vector3 forward { get => Vector3.forward; set { } }
        public Vector3 right { get => Vector3.right; set { } }
        public Vector3 up { get => Vector3.up; set { } }
        public int childCount => _children.Count;

        public void SetParent(Transform p, bool worldPositionStays = true)
        {
            parent?._children.Remove(this);
            parent = p;
            p?._children.Add(this);
        }
        public Transform GetChild(int index) => _children[index];
        public Transform Find(string n) => _children.Find(c => c.name == n);
        public bool IsChildOf(Transform p)
        {
            for (var cur = this; cur != null; cur = cur.parent)
                if (cur == p) return true;
            return false;
        }
        public void SetPositionAndRotation(Vector3 pos, Quaternion rot) { position = pos; rotation = rot; }
        public void LookAt(Transform target) { }
        public void LookAt(Vector3 worldPosition) { }
        public void LookAt(Vector3 worldPosition, Vector3 worldUp) { }
        public void Rotate(Vector3 eulers, Space relativeTo = Space.Self) { }
        public void Rotate(float xAngle, float yAngle, float zAngle, Space relativeTo = Space.Self) { }
        public void Rotate(Vector3 axis, float angle, Space relativeTo = Space.Self) { }
        public Vector3 TransformPoint(Vector3 p) => position + p;
        public Vector3 TransformDirection(Vector3 d) => d;
        public Vector3 InverseTransformPoint(Vector3 p) => p - position;
        public IEnumerator<Transform> GetEnumerator() => _children.GetEnumerator();
        IEnumerator IEnumerable.GetEnumerator() => _children.GetEnumerator();
    }

    public class RectTransform : Transform
    {
        public Vector2 anchorMin { get; set; }
        public Vector2 anchorMax { get; set; }
        public Vector2 anchoredPosition { get; set; }
        public Vector2 sizeDelta { get; set; }
        public Vector2 pivot { get; set; } = new Vector2(0.5f, 0.5f);
        public Vector2 offsetMin { get; set; }
        public Vector2 offsetMax { get; set; }
        public Rect rect => new Rect(0f, 0f, sizeDelta.x, sizeDelta.y);
    }

    public sealed class GameObject : Object
    {
        private readonly List<Component> _components = new List<Component>();
        public Transform transform { get; private set; }
        public bool activeSelf { get; private set; } = true;
        public bool activeInHierarchy
        {
            get
            {
                if (!activeSelf) return false;
                for (var p = transform?.parent; p != null; p = p.parent)
                {
                    if (p.gameObject != null && !p.gameObject.activeSelf)
                        return false;
                }
                return true;
            }
        }
        public int layer { get; set; }
        public string tag { get; set; } = "Untagged";

        public GameObject() : this("GameObject") { }
        public GameObject(string name)
        {
            this.name = name;
            var tr = new Transform { gameObject = this, name = name };
            transform = tr;
            _components.Add(tr);
        }
        public GameObject(string name, params Type[] components) : this(name)
        {
            if (components == null) return;
            foreach (var t in components) AddComponent(t);
        }

        public void SetActive(bool value) => activeSelf = value;

        public Component AddComponent(Type componentType)
        {
            if (typeof(RectTransform).IsAssignableFrom(componentType))
            {
                if (transform is RectTransform existingRt)
                    return existingRt;
                var prev = transform;
                var prevParent = prev?.parent;
                var rt = new RectTransform
                {
                    gameObject = this,
                    name = name,
                    position = prev != null ? prev.position : Vector3.zero,
                    localPosition = prev != null ? prev.localPosition : Vector3.zero,
                    rotation = prev != null ? prev.rotation : Quaternion.identity,
                    localRotation = prev != null ? prev.localRotation : Quaternion.identity,
                    localScale = prev != null ? prev.localScale : Vector3.one
                };
                if (prev != null)
                {
                    prev.SetParent(null, false);
                    while (prev.childCount > 0)
                        prev.GetChild(0).SetParent(rt, false);
                }
                if (prevParent != null)
                    rt.SetParent(prevParent, false);
                transform = rt;
                _components[0] = rt;
                return rt;
            }
            var inst = (Component)Activator.CreateInstance(componentType);
            inst.gameObject = this;
            inst.name = name;
            _components.Add(inst);
            if (inst is Camera cam)
                Camera.Register(cam);
            return inst;
        }

        public T AddComponent<T>() where T : Component, new() => (T)AddComponent(typeof(T));

        internal void RemoveComponentInternal(Component comp)
        {
            if (comp != null && comp != transform)
            {
                if (comp is Camera cam)
                    Camera.Unregister(cam);
                _components.Remove(comp);
            }
        }

        public T GetComponent<T>()
        {
            for (int i = 0; i < _components.Count; i++)
            {
                if (_components[i] is T match) return match;
            }
            return default;
        }
        public T[] GetComponents<T>()
        {
            var res = new List<T>();
            for (int i = 0; i < _components.Count; i++)
            {
                if (_components[i] is T match) res.Add(match);
            }
            return res.ToArray();
        }
        public T GetComponentInChildren<T>(bool includeInactive = false)
        {
            var all = GetComponentsInChildren<T>(includeInactive);
            return all.Length > 0 ? all[0] : default;
        }
        public T[] GetComponentsInChildren<T>(bool includeInactive = false)
        {
            var res = new List<T>();
            CollectComponentsRecursive(this, includeInactive, res);
            return res.ToArray();
        }
        private static void CollectComponentsRecursive<T>(GameObject go, bool includeInactive, List<T> acc)
        {
            if (go == null || (!includeInactive && !go.activeSelf)) return;
            for (int i = 0; i < go._components.Count; i++)
            {
                if (go._components[i] is T match) acc.Add(match);
            }
            if (go.transform == null) return;
            for (int c = 0; c < go.transform.childCount; c++)
            {
                CollectComponentsRecursive(go.transform.GetChild(c)?.gameObject, includeInactive, acc);
            }
        }

        public static GameObject CreatePrimitive(PrimitiveType type)
        {
            var go = new GameObject(type.ToString());
            go.AddComponent<MeshFilter>();
            go.AddComponent<MeshRenderer>();
            go.AddComponent<BoxCollider>();
            return go;
        }
    }

    public class Shader : Object
    {
        public bool isSupported => true;
        public static Shader Find(string name) => new Shader { name = name };
    }

    public class Material : Object
    {
        public Shader shader { get; set; }
        public Color color { get; set; } = Color.white;
        public Texture mainTexture { get; set; }
        public Vector2 mainTextureScale { get; set; } = Vector2.one;
        public Vector2 mainTextureOffset { get; set; } = Vector2.zero;
        public int renderQueue { get; set; }
        public Material(Shader shader) { this.shader = shader; }
        public Material(Material source) { this.shader = source?.shader; this.color = source != null ? source.color : Color.white; }
        public bool HasProperty(string name) => true;
        public Color GetColor(string name) => color;
        public float GetFloat(string name) => 0f;
        public Texture GetTexture(string name) => null;
        public Vector2 GetTextureScale(string name) => Vector2.one;
        public Vector2 GetTextureOffset(string name) => Vector2.zero;
        public bool IsKeywordEnabled(string keyword) => false;
        public void SetColor(string name, Color value) { }
        public void SetFloat(string name, float value) { }
        public void SetInt(string name, int value) { }
        public void SetVector(string name, Vector4 value) { }
        public void SetTexture(string name, Texture value) { }
        public void SetTextureScale(string name, Vector2 value) { }
        public void SetTextureOffset(string name, Vector2 value) { }
        public void EnableKeyword(string keyword) { }
        public void DisableKeyword(string keyword) { }
        public void SetOverrideTag(string tag, string val) { }
    }

    public enum TextureWrapMode { Repeat, Clamp, Mirror, MirrorOnce }
    public enum FilterMode { Point, Bilinear, Trilinear }
    public enum TextureFormat { RGB24, RGBA32, ARGB32 }

    public class Texture : Object
    {
        public int width { get; set; }
        public int height { get; set; }
        public int anisoLevel { get; set; }
        public TextureWrapMode wrapMode { get; set; }
        public FilterMode filterMode { get; set; }
    }
    public class Texture2D : Texture
    {
        public Texture2D(int width, int height) { this.width = width; this.height = height; }
        public Texture2D(int width, int height, TextureFormat textureFormat, bool mipChain) : this(width, height) { }
        public Texture2D(int width, int height, TextureFormat textureFormat, bool mipChain, bool linear) : this(width, height) { }
        public static Texture2D whiteTexture { get; } = new Texture2D(4, 4);
        public void SetPixel(int x, int y, Color color) { }
        public void SetPixels32(Color32[] colors) { }
        public void Apply(bool updateMipmaps = true, bool makeNoLongerReadable = false) { }
    }

    public class Sprite : Object
    {
        public static Sprite Create(Texture2D texture, Rect rect, Vector2 pivot, float pixelsPerUnit = 100f) => new Sprite();
    }

    public class Mesh : Object
    {
        public Rendering.IndexFormat indexFormat { get; set; }
        public Vector3[] vertices { get; set; }
        public Vector3[] normals { get; set; }
        public Vector2[] uv { get; set; }
        public Color[] colors { get; set; }
        public int[] triangles { get; set; }
        public Bounds bounds { get; set; }
        public void SetVertices(List<Vector3> inVertices) => vertices = inVertices?.ToArray();
        public void SetNormals(List<Vector3> inNormals) => normals = inNormals?.ToArray();
        public void SetUVs(int channel, List<Vector2> uvs) => uv = uvs?.ToArray();
        public void SetColors(List<Color> inColors) => colors = inColors?.ToArray();
        public void SetTriangles(List<int> triangles, int submesh) => this.triangles = triangles?.ToArray();
        public void RecalculateNormals() { }
        public void RecalculateBounds() { }
        public void Clear() { }
    }

    public class MeshFilter : Component
    {
        public Mesh sharedMesh { get; set; }
        public Mesh mesh { get => sharedMesh; set => sharedMesh = value; }
    }

    public class Renderer : Component
    {
        public bool enabled { get; set; } = true;
        public Material sharedMaterial { get; set; }
        public Material material { get => sharedMaterial; set => sharedMaterial = value; }
        public Material[] sharedMaterials { get; set; } = Array.Empty<Material>();
        public Bounds bounds { get; set; }
        public Rendering.ShadowCastingMode shadowCastingMode { get; set; }
        public bool receiveShadows { get; set; } = true;
    }

    public class MeshRenderer : Renderer { }
    public class SkinnedMeshRenderer : Renderer { }

    public class LineRenderer : Renderer
    {
        public int positionCount { get; set; }
        public float startWidth { get; set; }
        public float endWidth { get; set; }
        public Color startColor { get; set; }
        public Color endColor { get; set; }
        public bool useWorldSpace { get; set; } = true;
        public bool loop { get; set; }
        public void SetPosition(int index, Vector3 position) { }
        public void SetPositions(Vector3[] positions) { }
    }

    public class ParticleSystem : Component
    {
        public void Play() { }
        public void Stop() { }
    }
    public class ParticleSystemRenderer : Renderer { }

    public class Collider : Component
    {
        public bool enabled { get; set; } = true;
        public bool isTrigger { get; set; }
        public Bounds bounds { get; set; }
    }
    public class BoxCollider : Collider { public Vector3 center { get; set; } public Vector3 size { get; set; } = Vector3.one; }
    public class SphereCollider : Collider { public Vector3 center { get; set; } public float radius { get; set; } = 0.5f; }
    public class CapsuleCollider : Collider { public Vector3 center { get; set; } public float radius { get; set; } = 0.5f; public float height { get; set; } = 2f; }
    public class MeshCollider : Collider { public Mesh sharedMesh { get; set; } public bool convex { get; set; } }

    public class CharacterController : Collider
    {
        public float height { get; set; } = 1.8f;
        public float radius { get; set; } = 0.35f;
        public Vector3 center { get; set; }
        public float stepOffset { get; set; } = 0.3f;
        public float slopeLimit { get; set; } = 45f;
        public float skinWidth { get; set; } = 0.08f;
        public float minMoveDistance { get; set; }
        public bool isGrounded => true;
        public Vector3 velocity => Vector3.zero;
        public CollisionFlags Move(Vector3 motion) { transform.position += motion; return CollisionFlags.Below; }
    }

    public class Camera : Behaviour
    {
        private static readonly List<Camera> RegisteredCameras = new List<Camera>();

        internal static void Register(Camera cam)
        {
            if (cam != null && !RegisteredCameras.Contains(cam))
                RegisteredCameras.Add(cam);
        }

        internal static void Unregister(Camera cam)
        {
            if (cam != null)
                RegisteredCameras.Remove(cam);
        }

        public static Camera main
        {
            get
            {
                for (int i = 0; i < RegisteredCameras.Count; i++)
                {
                    var c = RegisteredCameras[i];
                    if (c != null && c.enabled && c.gameObject != null &&
                        c.gameObject.activeInHierarchy && c.gameObject.tag == "MainCamera")
                    {
                        return c;
                    }
                }
                return null;
            }
        }
        public float fieldOfView { get; set; } = 60f;
        public float nearClipPlane { get; set; } = 0.1f;
        public float farClipPlane { get; set; } = 2000f;
        public CameraClearFlags clearFlags { get; set; }
        public Color backgroundColor { get; set; }
        public int cullingMask { get; set; } = -1;
        public float depth { get; set; }
        public Ray ScreenPointToRay(Vector3 pos) => new Ray(transform.position, Vector3.forward);
        public Vector3 WorldToScreenPoint(Vector3 position) => new Vector3(960f, 540f, 10f);
    }

    public class Light : Behaviour
    {
        public LightType type { get; set; }
        public Color color { get; set; } = Color.white;
        public float intensity { get; set; } = 1f;
        public float range { get; set; } = 10f;
        public float spotAngle { get; set; } = 45f;
        public LightShadows shadows { get; set; }
        public float shadowStrength { get; set; } = 1f;
    }

    public enum AnimatorCullingMode { AlwaysAnimate = 0, CullUpdateTransforms = 1, CullCompletely = 2 }
    public enum HumanBodyBones
    {
        Hips, LeftUpperLeg, RightUpperLeg, LeftLowerLeg, RightLowerLeg, LeftFoot, RightFoot,
        Spine, Chest, UpperChest, Neck, Head,
        LeftShoulder, RightShoulder, LeftUpperArm, RightUpperArm, LeftLowerArm, RightLowerArm, LeftHand, RightHand
    }

    public class Animator : Behaviour
    {
        public RuntimeAnimatorController runtimeAnimatorController { get; set; }
        public Avatar avatar { get; set; }
        public bool applyRootMotion { get; set; }
        public AnimatorCullingMode cullingMode { get; set; }
        public bool isHuman => avatar != null && avatar.isHuman;
        public float speed { get; set; } = 1f;
        public static int StringToHash(string name) => name?.GetHashCode() ?? 0;
        public Transform GetBoneTransform(HumanBodyBones humanBoneId) => transform;
        public void Play(string stateName, int layer = -1, float normalizedTime = float.NegativeInfinity) { }
        public void CrossFadeInFixedTime(string stateName, float fixedTransitionDuration, int layer = -1) { }
        public void Update(float deltaTime) { }
        public void SetFloat(string name, float value) { }
        public void SetFloat(int id, float value) { }
        public void SetFloat(int id, float value, float dampTime, float deltaTime) { }
        public void SetBool(string name, bool value) { }
        public void SetBool(int id, bool value) { }
        public void SetTrigger(string name) { }
        public void SetTrigger(int id) { }
    }
    public class RuntimeAnimatorController : Object
    {
        public AnimationClip[] animationClips { get; set; } = Array.Empty<AnimationClip>();
    }
    public class Avatar : Object { public bool isValid => true; public bool isHuman => true; }
    public enum AnimatorControllerParameterType { Float = 1, Int = 3, Bool = 4, Trigger = 9 }
    public class Motion : Object { }
    public class AnimationClip : Motion
    {
        public void SetCurve(string relativePath, Type type, string propertyName, AnimationCurve curve) { }
    }
    public struct Keyframe
    {
        public float time, value;
        public Keyframe(float t, float v) { time = t; value = v; }
    }
    public class AnimationCurve
    {
        public int AddKey(Keyframe key) => 0;
        public static AnimationCurve Constant(float timeStart, float timeEnd, float value) => new AnimationCurve();
    }

    public class AudioClip : Object
    {
        public float length { get; private set; }
        public static AudioClip Create(string name, int lengthSamples, int channels, int frequency, bool stream) =>
            new AudioClip { name = name, length = frequency > 0 ? (float)lengthSamples / frequency : 0f };
        public bool SetData(float[] data, int offsetSamples) => true;
    }

    public class AudioListener : Behaviour { }

    public class AudioSource : Behaviour
    {
        public AudioClip clip { get; set; }
        public float volume { get; set; } = 1f;
        public float pitch { get; set; } = 1f;
        public bool loop { get; set; }
        public bool playOnAwake { get; set; }
        public float spatialBlend { get; set; }
        public float minDistance { get; set; } = 1f;
        public float maxDistance { get; set; } = 50f;
        public AudioRolloffMode rolloffMode { get; set; }
        public bool isPlaying { get; private set; }
        public void Play() => isPlaying = true;
        public void Stop() => isPlaying = false;
        public void PlayOneShot(AudioClip clip, float volumeScale = 1f) { }
    }

    public enum RenderMode { ScreenSpaceOverlay, ScreenSpaceCamera, WorldSpace }
    public class Canvas : Behaviour
    {
        public RenderMode renderMode { get; set; }
        public int sortingOrder { get; set; }
        public Camera worldCamera { get; set; }
    }
    public class CanvasGroup : Behaviour
    {
        public float alpha { get; set; } = 1f;
        public bool interactable { get; set; } = true;
        public bool blocksRaycasts { get; set; } = true;
    }
    public class Font : Object
    {
        public static Font CreateDynamicFontFromOSFont(string fontname, int size) => new Font { name = fontname };
        public static Font CreateDynamicFontFromOSFont(string[] fontnames, int size) => new Font();
    }

    namespace Rendering
    {
        public enum IndexFormat { UInt16 = 0, UInt32 = 1 }
        public enum ShadowCastingMode { Off, On, TwoSided, ShadowsOnly }
        public enum AmbientMode { Skybox, Trilight, Flat, Custom }
        public enum BlendMode { Zero = 0, One = 1, DstColor = 2, SrcColor = 3, OneMinusDstColor = 4, SrcAlpha = 5, OneMinusSrcColor = 6, DstAlpha = 7, OneMinusDstAlpha = 8, SrcAlphaSaturate = 9, OneMinusSrcAlpha = 10 }
        public enum RenderQueue { Background = 1000, Geometry = 2000, AlphaTest = 2450, GeometryLast = 2500, Transparent = 3000, Overlay = 4000 }
        public class RenderPipelineAsset : ScriptableObject { }
        public static class GraphicsSettings
        {
            public static RenderPipelineAsset defaultRenderPipeline { get; set; }
        }
        public class VolumeProfile : ScriptableObject { }
        public class Volume : Behaviour
        {
            public bool isGlobal { get; set; }
            public VolumeProfile sharedProfile { get; set; }
        }

        namespace Universal
        {
            public class UniversalRenderPipelineAsset : RenderPipelineAsset { }
        }
    }

    namespace EventSystems
    {
        public class UIBehaviour : MonoBehaviour { }
        public class EventSystem : UIBehaviour
        {
            public static EventSystem current { get; set; }
            public GameObject currentSelectedGameObject { get; set; }
            public bool IsPointerOverGameObject() => false;
            public void SetSelectedGameObject(GameObject selected) => currentSelectedGameObject = selected;
        }
        public class BaseInputModule : UIBehaviour { }
        public class StandaloneInputModule : BaseInputModule { }
        public interface IPointerClickHandler { }
    }

    namespace UI
    {
        public sealed class UnityEvent
        {
            private Action _listeners;
            public void AddListener(Action call) => _listeners += call;
            public void RemoveAllListeners() => _listeners = null;
            public void Invoke() => _listeners?.Invoke();
        }

        public sealed class UnityEvent<T>
        {
            private Action<T> _listeners;
            public void AddListener(Action<T> call) => _listeners += call;
            public void RemoveAllListeners() => _listeners = null;
            public void Invoke(T arg) => _listeners?.Invoke(arg);
        }

        public class Graphic : EventSystems.UIBehaviour
        {
            public Color color { get; set; } = Color.white;
            public bool raycastTarget { get; set; } = true;
            public RectTransform rectTransform => transform as RectTransform ?? gameObject?.AddComponent<RectTransform>();
        }

        public class MaskableGraphic : Graphic { }

        public class Image : MaskableGraphic
        {
            public enum Type { Simple, Sliced, Tiled, Filled }
            public enum FillMethod { Horizontal, Vertical, Radial90, Radial180, Radial360 }
            public Sprite sprite { get; set; }
            public Type type { get; set; }
            public FillMethod fillMethod { get; set; }
            public float fillAmount { get; set; } = 1f;
        }

        public class RawImage : MaskableGraphic { public Texture texture { get; set; } }

        public class Text : MaskableGraphic
        {
            public virtual string text { get; set; } = string.Empty;
            public Font font { get; set; }
            public int fontSize { get; set; } = 14;
            public FontStyle fontStyle { get; set; }
            public TextAnchor alignment { get; set; }
            public bool supportRichText { get; set; } = true;
            public HorizontalWrapMode horizontalOverflow { get; set; }
            public VerticalWrapMode verticalOverflow { get; set; }
            public float lineSpacing { get; set; } = 1f;
        }

        public struct ColorBlock
        {
            public Color normalColor { get; set; }
            public Color highlightedColor { get; set; }
            public Color pressedColor { get; set; }
            public Color selectedColor { get; set; }
            public Color disabledColor { get; set; }
            public float colorMultiplier { get; set; }
            public float fadeDuration { get; set; }
            public static ColorBlock defaultColorBlock => new ColorBlock
            {
                normalColor = Color.white,
                highlightedColor = Color.white,
                pressedColor = Color.gray,
                selectedColor = Color.white,
                disabledColor = Color.gray,
                colorMultiplier = 1f,
                fadeDuration = 0.1f
            };
        }

        public class Selectable : EventSystems.UIBehaviour
        {
            public bool interactable { get; set; } = true;
            public Graphic targetGraphic { get; set; }
            public Image image { get; set; }
            public ColorBlock colors { get; set; } = ColorBlock.defaultColorBlock;
            public virtual void Select() { }
        }

        public class Button : Selectable
        {
            public UnityEvent onClick { get; } = new UnityEvent();
        }

        public class InputField : Selectable
        {
            public enum ContentType { Standard, Autocorrected, IntegerNumber, DecimalNumber, Alphanumeric, Name, EmailAddress, Password, Pin, Custom }
            public enum LineType { SingleLine, MultiLineSubmit, MultiLineNewline }
            private string _text = string.Empty;
            private bool _isFocused;
            public string text
            {
                get => _text;
                set => SetText(value, sendCallback: true);
            }
            public Text textComponent { get; set; }
            public Graphic placeholder { get; set; }
            public ContentType contentType { get; set; }
            public LineType lineType { get; set; }
            public int characterLimit { get; set; }
            public Color caretColor { get; set; }
            public Color selectionColor { get; set; }
            public bool customCaretColor { get; set; }
            public bool isFocused => _isFocused;
            public UnityEvent<string> onValueChanged { get; } = new UnityEvent<string>();
            public UnityEvent<string> onEndEdit { get; } = new UnityEvent<string>();
            public UnityEvent<string> onSubmit { get; } = new UnityEvent<string>();
            public void SetTextWithoutNotify(string input) => SetText(input, sendCallback: false);
            public void ActivateInputField() => _isFocused = true;
            public void DeactivateInputField() => _isFocused = false;
            public override void Select() => _isFocused = true;

            private void SetText(string value, bool sendCallback)
            {
                // Real Unity uGUI InputField normalizes null to empty, strips embedded NULs,
                // and invokes onValueChanged whenever the normalized text changes.
                string normalized = (value ?? string.Empty).Replace("\0", string.Empty);
                if (_text == normalized)
                    return;
                _text = normalized;
                if (sendCallback)
                    onValueChanged.Invoke(_text);
            }
        }

        public class ScrollRect : EventSystems.UIBehaviour
        {
            public enum MovementType { Unrestricted, Elastic, Clamped }
            public RectTransform content { get; set; }
            public RectTransform viewport { get; set; }
            public bool horizontal { get; set; }
            public bool vertical { get; set; } = true;
            public MovementType movementType { get; set; }
            public float scrollSensitivity { get; set; } = 1f;
            public Vector2 normalizedPosition { get; set; }
            public float verticalNormalizedPosition { get; set; }
        }

        public class Mask : EventSystems.UIBehaviour { public bool showMaskGraphic { get; set; } }
        public class RectMask2D : EventSystems.UIBehaviour { }

        public class CanvasScaler : EventSystems.UIBehaviour
        {
            public enum ScaleMode { ConstantPixelSize, ScaleWithScreenSize, ConstantPhysicalSize }
            public enum ScreenMatchMode { MatchWidthOrHeight, Expand, Shrink }
            public ScaleMode uiScaleMode { get; set; }
            public Vector2 referenceResolution { get; set; }
            public ScreenMatchMode screenMatchMode { get; set; }
            public float matchWidthOrHeight { get; set; }
        }

        public class GraphicRaycaster : EventSystems.UIBehaviour { }

        public struct RectOffset
        {
            public int left, right, top, bottom;
            public RectOffset(int l, int r, int t, int b) { left = l; right = r; top = t; bottom = b; }
        }

        public abstract class LayoutGroup : EventSystems.UIBehaviour
        {
            public RectOffset padding { get; set; }
            public TextAnchor childAlignment { get; set; }
        }

        public abstract class HorizontalOrVerticalLayoutGroup : LayoutGroup
        {
            public float spacing { get; set; }
            public bool childForceExpandWidth { get; set; }
            public bool childForceExpandHeight { get; set; }
            public bool childControlWidth { get; set; }
            public bool childControlHeight { get; set; }
        }

        public class VerticalLayoutGroup : HorizontalOrVerticalLayoutGroup { }
        public class HorizontalLayoutGroup : HorizontalOrVerticalLayoutGroup { }
        public class GridLayoutGroup : LayoutGroup
        {
            public Vector2 cellSize { get; set; }
            public Vector2 spacing { get; set; }
        }

        public class ContentSizeFitter : EventSystems.UIBehaviour
        {
            public enum FitMode { Unconstrained, MinSize, PreferredSize }
            public FitMode horizontalFit { get; set; }
            public FitMode verticalFit { get; set; }
        }

        public class LayoutElement : EventSystems.UIBehaviour
        {
            public float minWidth { get; set; }
            public float minHeight { get; set; }
            public float preferredWidth { get; set; }
            public float preferredHeight { get; set; }
            public float flexibleWidth { get; set; }
            public float flexibleHeight { get; set; }
        }

        public static class LayoutRebuilder
        {
            public static void ForceRebuildLayoutImmediate(RectTransform layoutRoot) { }
        }

        public class Outline : EventSystems.UIBehaviour
        {
            public Color effectColor { get; set; }
            public Vector2 effectDistance { get; set; }
        }
        public class Shadow : Outline { }
    }

    namespace Networking
    {
        public class DownloadHandler : IDisposable
        {
            public string text { get; set; } = string.Empty;
            public byte[] data { get; set; } = Array.Empty<byte>();
            public void Dispose() { }
        }
        public class DownloadHandlerBuffer : DownloadHandler { }
        public class UploadHandler : IDisposable { public void Dispose() { } }
        public class UploadHandlerRaw : UploadHandler
        {
            public string contentType { get; set; }
            public UploadHandlerRaw(byte[] data) { }
        }

        public sealed class UnityWebRequestAsyncOperation
        {
            public bool isDone => true;
            public event Action<UnityWebRequestAsyncOperation> completed
            {
                add => value?.Invoke(this);
                remove { }
            }
        }

        public class UnityWebRequest : IDisposable
        {
            public enum Result { InProgress, Success, ConnectionError, ProtocolError, DataProcessingError }
            public const string kHttpVerbGET = "GET";
            public const string kHttpVerbPOST = "POST";

            public string url { get; set; }
            public string method { get; set; }
            public UploadHandler uploadHandler { get; set; }
            public DownloadHandler downloadHandler { get; set; }
            public int timeout { get; set; }
            public long responseCode { get; set; } = 200;
            public Result result { get; set; } = Result.Success;
            public string error { get; set; }

            public UnityWebRequest() { }
            public UnityWebRequest(string url, string method) { this.url = url; this.method = method; }
            public static UnityWebRequest Get(string uri) => new UnityWebRequest(uri, kHttpVerbGET) { downloadHandler = new DownloadHandlerBuffer() };
            public void SetRequestHeader(string name, string value) { }
            public string GetResponseHeader(string name) => null;
            public Dictionary<string, string> GetResponseHeaders() => new Dictionary<string, string>();
            public UnityWebRequestAsyncOperation SendWebRequest() => new UnityWebRequestAsyncOperation();
            public void Dispose() { uploadHandler?.Dispose(); downloadHandler?.Dispose(); }
        }
    }
}

namespace TMPro
{
    public enum TextAlignmentOptions { TopLeft, Top, TopRight, Left, Center, Right, BottomLeft, Bottom, BottomRight }
    public enum FontStyles { Normal = 0, Bold = 1, Italic = 2 }
    public enum TextWrappingModes { NoWrap = 0, Normal = 1, PreserveWhitespace = 2, PreserveWhitespaceNoWrap = 3 }
    public enum TextOverflowModes { Overflow = 0, Ellipsis = 1, Masking = 2, Truncate = 3 }
    public class TMP_Text : UnityEngine.UI.MaskableGraphic
    {
        public string text { get; set; } = string.Empty;
        public float fontSize { get; set; } = 24f;
        public FontStyles fontStyle { get; set; }
        public TextAlignmentOptions alignment { get; set; }
        public bool enableWordWrapping { get; set; }
        public TextWrappingModes textWrappingMode { get; set; }
        public TextOverflowModes overflowMode { get; set; }
        public bool richText { get; set; } = true;
    }
    public class TextMeshPro : TMP_Text { }
    public class TextMeshProUGUI : TMP_Text { }
}

namespace UnityEngine.SceneManagement
{
    public struct Scene { }
}

namespace UnityEditor
{
    public sealed class InitializeOnLoadAttribute : Attribute { }
    public sealed class MenuItemAttribute : Attribute
    {
        public int priority;
        public MenuItemAttribute(string itemName) { }
        public MenuItemAttribute(string itemName, bool isValidateFunction) { }
        public MenuItemAttribute(string itemName, bool isValidateFunction, int priority) { this.priority = priority; }
    }

    public static class EditorApplication
    {
        public delegate void CallbackFunction();
        public static CallbackFunction delayCall;
        public static bool isPlayingOrWillChangePlaymode => false;
    }

    public static class EditorPrefs
    {
        public static string GetString(string key, string defaultValue = "") => defaultValue;
        public static void SetString(string key, string value) { }
    }

    public static class EditorUtility
    {
        public static void SetDirty(UnityEngine.Object target) { }
        public static void CopySerialized(UnityEngine.Object source, UnityEngine.Object dest) { }
        public static string OpenFolderPanel(string title, string folder, string defaultName) => string.Empty;
        public static string SaveFilePanel(string title, string directory, string defaultName, string extension) => string.Empty;
        public static void RevealInFinder(string path) { }
        public static bool DisplayDialog(string title, string message, string ok, string cancel = "") => true;
    }

    public enum BuildTarget
    {
        StandaloneOSX = 2,
        StandaloneWindows64 = 19,
        StandaloneLinux64 = 24
    }

    [Flags]
    public enum BuildOptions
    {
        None = 0,
        Development = 1,
        StrictMode = 512,
        CompressWithLz4 = 4194304
    }

    public struct BuildPlayerOptions
    {
        public string[] scenes { get; set; }
        public string locationPathName { get; set; }
        public BuildTarget target { get; set; }
        public BuildOptions options { get; set; }
    }

    public static class BuildPipeline
    {
        public static Build.Reporting.BuildReport BuildPlayer(BuildPlayerOptions buildPlayerOptions) => new Build.Reporting.BuildReport();
    }

    namespace Build
    {
        public class BuildFailedException : Exception
        {
            public BuildFailedException(string message) : base(message) { }
            public BuildFailedException(Exception innerException) : base(innerException?.Message, innerException) { }
        }

        public interface IOrderedCallback
        {
            int callbackOrder { get; }
        }

        public interface IPreprocessBuildWithReport : IOrderedCallback
        {
            void OnPreprocessBuild(Reporting.BuildReport report);
        }

        namespace Reporting
        {
            public enum BuildResult { Unknown = 0, Succeeded = 1, Failed = 2, Cancelled = 3 }

            public struct BuildSummary
            {
                public BuildResult result;
                public ulong totalSize;
                public TimeSpan totalTime;
                public int totalErrors;
                public int totalWarnings;
                public string outputPath;
            }

            public class BuildReport : UnityEngine.Object
            {
                public BuildSummary summary { get; set; }
            }
        }
    }

    [Flags]
    public enum ImportAssetOptions
    {
        Default = 0,
        ForceUpdate = 1,
        ForceSynchronousImport = 8,
        ImportRecursive = 256
    }

    public static class AssetDatabase
    {
        public static T LoadAssetAtPath<T>(string assetPath) where T : UnityEngine.Object => null;
        public static UnityEngine.Object[] LoadAllAssetsAtPath(string assetPath) => Array.Empty<UnityEngine.Object>();
        public static string[] FindAssets(string filter) => Array.Empty<string>();
        public static string GUIDToAssetPath(string guid) => string.Empty;
        public static string GetAssetPath(UnityEngine.Object assetObject) => string.Empty;
        public static void CreateAsset(UnityEngine.Object asset, string path) { }
        public static bool IsValidFolder(string path) => true;
        public static string CreateFolder(string parentFolder, string newFolderName) => string.Empty;
        public static void SaveAssets() { }
        public static void Refresh(ImportAssetOptions options = ImportAssetOptions.Default) { }
    }

    public class AssetImporter : UnityEngine.Object
    {
        public static AssetImporter GetAtPath(string path) => null;
        public void SaveAndReimport() { }
    }

    public enum ModelImporterAnimationType { None = 0, Legacy = 1, Generic = 2, Human = 3 }

    public class ModelImporterClipAnimation
    {
        public string name { get; set; }
        public bool loopTime { get; set; }
        public bool loopPose { get; set; }
        public bool keepOriginalPositionY { get; set; }
        public bool keepOriginalPositionXZ { get; set; }
        public bool keepOriginalOrientation { get; set; }
    }

    public class ModelImporter : AssetImporter
    {
        public ModelImporterAnimationType animationType { get; set; }
        public bool importAnimation { get; set; } = true;
        public ModelImporterClipAnimation[] defaultClipAnimations { get; set; } = Array.Empty<ModelImporterClipAnimation>();
        public ModelImporterClipAnimation[] clipAnimations { get; set; } = Array.Empty<ModelImporterClipAnimation>();
    }

    public class SerializedProperty
    {
        public bool isArray => false;
        public int arraySize => 0;
        public string stringValue { get; set; }
        public bool boolValue { get; set; }
        public UnityEngine.Object objectReferenceValue { get; set; }
        public SerializedProperty GetArrayElementAtIndex(int index) => new SerializedProperty();
        public SerializedProperty FindPropertyRelative(string relativePropertyPath) => new SerializedProperty();
    }

    public class SerializedObject
    {
        public SerializedObject(UnityEngine.Object obj) { }
        public SerializedProperty FindProperty(string propertyPath) => new SerializedProperty();
        public bool ApplyModifiedPropertiesWithoutUndo() => true;
    }

    public class EditorBuildSettingsScene
    {
        public EditorBuildSettingsScene(string path, bool enabled) { }
    }

    public static class EditorBuildSettings
    {
        public static EditorBuildSettingsScene[] scenes { get; set; } = Array.Empty<EditorBuildSettingsScene>();
    }

    namespace AssetPackage
    {
        public static class Package
        {
            public static void Import(string packagePath, bool interactive = false) { }
        }
    }

    namespace SceneManagement
    {
        public enum NewSceneSetup { EmptyScene, DefaultGameObjects }
        public enum NewSceneMode { Single, Additive }
        public static class EditorSceneManager
        {
            public static UnityEngine.SceneManagement.Scene NewScene(NewSceneSetup setup, NewSceneMode mode = NewSceneMode.Single) => default;
            public static bool SaveScene(UnityEngine.SceneManagement.Scene scene, string dstScenePath = "") => true;
        }
    }

    namespace Animations
    {
        public enum AnimatorConditionMode { If = 1, IfNot = 2, Greater = 3, Less = 4, Equals = 6, NotEqual = 7 }

        public class AnimatorStateTransition : UnityEngine.Object
        {
            public bool hasExitTime { get; set; }
            public bool canTransitionToSelf { get; set; }
            public float duration { get; set; }
            public void AddCondition(AnimatorConditionMode mode, float threshold, string parameter) { }
        }

        public class AnimatorState : UnityEngine.Object
        {
            public UnityEngine.Motion motion { get; set; }
            public AnimatorStateTransition AddTransition(AnimatorState destinationState) => new AnimatorStateTransition();
        }

        public struct ChildAnimatorState
        {
            public AnimatorState state { get; set; }
            public UnityEngine.Vector3 position { get; set; }
        }

        public class AnimatorStateMachine : UnityEngine.Object
        {
            private readonly List<ChildAnimatorState> _states = new List<ChildAnimatorState>();
            public ChildAnimatorState[] states { get => _states.ToArray(); set { _states.Clear(); if (value != null) _states.AddRange(value); } }
            public AnimatorState defaultState { get; set; }
            public AnimatorState AddState(string name, UnityEngine.Vector3 position)
            {
                var st = new AnimatorState { name = name };
                _states.Add(new ChildAnimatorState { state = st, position = position });
                return st;
            }
            public AnimatorStateTransition AddAnyStateTransition(AnimatorState destinationState) => new AnimatorStateTransition();
        }

        public class AnimatorControllerLayer
        {
            public AnimatorStateMachine stateMachine { get; set; } = new AnimatorStateMachine();
        }

        public class AnimatorController : UnityEngine.RuntimeAnimatorController
        {
            public AnimatorControllerLayer[] layers { get; set; } = new[] { new AnimatorControllerLayer() };
            public static AnimatorController CreateAnimatorControllerAtPath(string path) => new AnimatorController();
            public void AddParameter(string name, UnityEngine.AnimatorControllerParameterType type) { }
        }
    }
}

