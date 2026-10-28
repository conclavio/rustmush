Shader "Hidden/NormalOut" {
  SubShader { Pass { Cull Back ZWrite On
    CGPROGRAM
    #pragma vertex v
    #pragma fragment f
    #include "UnityCG.cginc"
    struct a { float4 p : POSITION; float3 n : NORMAL; };
    struct o { float4 p : SV_POSITION; float3 n : TEXCOORD0; };
    o v(a i) { o r; r.p = UnityObjectToClipPos(i.p); r.n = UnityObjectToWorldNormal(i.n); return r; }
    float4 f(o i) : SV_Target { return float4(normalize(i.n) * 0.5 + 0.5, 1); }
    ENDCG } }
}
