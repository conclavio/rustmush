Shader "Hidden/IdOut" {
  Properties { _MainTex ("t", 2D) = "white" {} }
  SubShader { Pass { Cull Back ZWrite On
    CGPROGRAM
    #pragma vertex v
    #pragma fragment f
    #include "UnityCG.cginc"
    sampler2D _MainTex;
    struct a { float4 p : POSITION; float2 uv : TEXCOORD0; };
    struct o { float4 p : SV_POSITION; float2 uv : TEXCOORD0; };
    o v(a i) { o r; r.p = UnityObjectToClipPos(i.p); r.uv = i.uv; return r; }
    float4 f(o i) : SV_Target { return float4(tex2Dlod(_MainTex, float4(i.uv, 0, 0)).rgb, 1); }
    ENDCG } }
}
