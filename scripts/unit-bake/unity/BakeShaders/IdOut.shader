Shader "Hidden/IdOut" {
  Properties {
    _MainTex ("t", 2D) = "white" {}
    _AlbedoId ("classify an albedo texture into parts", Float) = 0
    _AlbedoMode ("albedo classification palette mode", Float) = 0
    _PaintLum ("luminance above which greyscale albedo is paint", Float) = 0
    _DarkLum ("luminance below which albedo is dark metal", Float) = 0.26
  }
  SubShader { Pass { Cull Back ZWrite On
    CGPROGRAM
    #pragma vertex v
    #pragma fragment f
    #include "UnityCG.cginc"
    sampler2D _MainTex; float _AlbedoId; float _AlbedoMode; float _PaintLum; float _DarkLum;
    struct a { float4 p : POSITION; float2 uv : TEXCOORD0; };
    struct o { float4 p : SV_POSITION; float2 uv : TEXCOORD0; };
    o v(a i) { o r; r.p = UnityObjectToClipPos(i.p); r.uv = i.uv; return r; }
    float4 f(o i) : SV_Target {
      float3 c = tex2Dlod(_MainTex, float4(i.uv, 0, 0)).rgb;
      if (_AlbedoId > 0.5) {
        float mx = max(c.r, max(c.g, c.b));
        float mn = min(c.r, min(c.g, c.b));
        float sat = mx > 0.01 ? (mx - mn) / mx : 0.0;
        float lum = dot(c, float3(0.299, 0.587, 0.114));
        float darkLum = _DarkLum > 0.001 ? _DarkLum : 0.26;
        if (_AlbedoMode > 1.5) {
          if ((c.r - c.g > 0.25 && sat > 0.45 && mx > 0.35) || (c.r - c.b > 0.35 && c.g - c.b > 0.30 && mx > 0.50)) c = float3(1, 0, 0);
          else if (c.b - c.r > 0.20 && c.g > 0.45) c = float3(1, 1, 1);
          else if (lum < darkLum) c = float3(0, 0, 0);
          else if (c.r - c.b > 0.06 && c.g >= c.b) c = float3(0, 1, 0);
          else if (_PaintLum > 0.01 && lum > _PaintLum) c = float3(0, 1, 0);
          else c = float3(0, 0, 1);
        } else if (_AlbedoMode > 0.5) {
          if ((lum > 0.62 && sat < 0.25) || (c.r - c.b > 0.35 && c.g - c.b > 0.30 && mx > 0.50)) c = float3(1, 0, 0);
          else if (c.b - c.r > 0.20 && c.g > 0.45) c = float3(1, 1, 1);
          else if (lum < darkLum) c = float3(0, 0, 0);
          else if (c.r - c.b > 0.15 && c.r >= c.g && mx > 0.22) c = float3(0, 1, 0);
          else if (_PaintLum > 0.01 && lum > _PaintLum) c = float3(0, 1, 0);
          else c = float3(0, 0, 1);
        } else {
          if (c.g - c.r > 0.012 && c.g - c.b > 0.012 && mx > 0.12) c = float3(0, 1, 0);
          else if (c.r - c.b > 0.28 && sat > 0.45 && mx > 0.42) c = float3(1, 0, 0);
          else if (c.b - c.r > 0.25 && c.g > 0.55) c = float3(1, 1, 1);
          else if (c.b - c.r > 0.035 && c.b >= c.g && mx > 0.22) c = float3(0, 1, 0);
          else if (_PaintLum > 0.01 && lum > _PaintLum) c = float3(0, 1, 0);
          else if (lum < darkLum) c = float3(0, 0, 0);
          else c = float3(0, 0, 1);
        }
      }
      return float4(c, 1);
    }
    ENDCG } }
}
