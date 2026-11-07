Shader "Hidden/LitOut" {
  Properties {
    _MainTex ("albedo", 2D) = "gray" {}
    _HasAlbedo ("has an albedo", Float) = 0
    _BumpMap ("normal map", 2D) = "bump" {}
    _HasBump ("has a normal map", Float) = 0
  }
  SubShader { Pass { Cull Back ZWrite On
    CGPROGRAM
    #pragma vertex v
    #pragma fragment f
    #include "UnityCG.cginc"
    sampler2D _MainTex; float _HasAlbedo; sampler2D _BumpMap; float _HasBump;
    float4 _KeyDir; float4 _FillDir;
    struct a { float4 p : POSITION; float3 n : NORMAL; float4 t : TANGENT; float2 uv : TEXCOORD0; };
    struct o { float4 p : SV_POSITION; float2 uv : TEXCOORD0; float3 n : TEXCOORD1; float3 t : TEXCOORD2; float3 b : TEXCOORD3; };
    o v(a i) {
      o r; r.p = UnityObjectToClipPos(i.p); r.uv = i.uv;
      r.n = UnityObjectToWorldNormal(i.n); r.t = UnityObjectToWorldDir(i.t.xyz);
      r.b = cross(r.n, r.t) * i.t.w * unity_WorldTransformParams.w;
      return r;
    }
    float4 f(o i) : SV_Target {
      float3 n = normalize(i.n);
      if (_HasBump > 0.5) {
        float3 tn = UnpackNormal(tex2Dlod(_BumpMap, float4(i.uv, 0, 0)));
        n = normalize(tn.x * normalize(i.t) + tn.y * normalize(i.b) + tn.z * n);
      }
      float key = saturate(dot(n, normalize(_KeyDir.xyz)));
      float fill = saturate(dot(n, normalize(_FillDir.xyz)));
      float lit = 0.18 + 0.72 * key + 0.22 * fill;
      float3 alb = _HasAlbedo > 0.5 ? tex2Dlod(_MainTex, float4(i.uv, 0, 0)).rgb : float3(0.5, 0.5, 0.5);
      float lum = dot(alb, float3(0.299, 0.587, 0.114));
      return float4(saturate(lit), lum, 0, 1);
    }
    ENDCG } }
}
