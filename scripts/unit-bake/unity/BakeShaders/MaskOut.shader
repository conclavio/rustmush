Shader "Hidden/MaskOut" {
  Properties {
    _MetallicGlossMap ("metallic (r), smoothness (a)", 2D) = "white" {}
    _HasMetalMap ("has a metallic map", Float) = 0
    _Metallic ("metallic without a map", Float) = 0
    _EmissionMap ("emission", 2D) = "black" {}
    _HasEmisMap ("has an emission map", Float) = 0
    _EmissionColor ("emission colour", Color) = (0, 0, 0, 1)
    _Rubber ("the material is rubber", Float) = 0
  }
  SubShader { Pass { Cull Back ZWrite On
    CGPROGRAM
    #pragma vertex v
    #pragma fragment f
    #include "UnityCG.cginc"
    sampler2D _MetallicGlossMap; float _HasMetalMap; float _Metallic;
    sampler2D _EmissionMap; float _HasEmisMap; float4 _EmissionColor; float _Rubber;
    struct a { float4 p : POSITION; float2 uv : TEXCOORD0; };
    struct o { float4 p : SV_POSITION; float2 uv : TEXCOORD0; };
    o v(a i) { o r; r.p = UnityObjectToClipPos(i.p); r.uv = i.uv; return r; }
    float4 f(o i) : SV_Target {
      float4 mg = tex2Dlod(_MetallicGlossMap, float4(i.uv, 0, 0));
      float metallic = _HasMetalMap > 0.5 ? mg.r : _Metallic;
      float3 e = _EmissionColor.rgb * (_HasEmisMap > 0.5 ? tex2Dlod(_EmissionMap, float4(i.uv, 0, 0)).rgb : float3(1, 1, 1));
      float emission = saturate(max(e.r, max(e.g, e.b)));
      return float4(metallic, _Rubber, emission, 1);
    }
    ENDCG } }
}
