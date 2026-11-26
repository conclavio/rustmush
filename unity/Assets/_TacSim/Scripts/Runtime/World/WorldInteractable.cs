using System;
using TMPro;
using UnityEngine;

namespace TacSim.World
{
    public enum InteractableKind
    {
        GenericCommand,
        RoomDetailPlaque,
        NpcActor,
        AirlockBoard,
        AirlockDisembark,
        BridgeHelmSeat,
        StationServiceDesk,
        SimulatorPodBoard,
        VehicleBayRig
    }

    /// <summary>
    /// Diegetic world-space [E] / click interaction target (NPCs, airlocks, bridge helm couches,
    /// station boards, Exchange &amp; Co-op counters) replacing manual +command memorization.
    /// </summary>
    public sealed class WorldInteractable : MonoBehaviour
    {
        public InteractableKind kind = InteractableKind.GenericCommand;
        public string promptTitle = "Inspect";
        public string promptSubtitle = "";
        public string commandOrKey = "";
        public string detailBody = "";
        public float maxInteractDistance = 3.4f;
        public bool isRemotePlayer;
        public string actorId = "";

        public Action<WorldInteractable> OnInteracted;

        public void TriggerInteraction()
        {
            OnInteracted?.Invoke(this);
        }
    }

    /// <summary>
    /// Metadata component on doorway/stairway visual portals. Actual 3D-to-MUSH room transitions are
    /// driven by continuous room-footprint hysteresis + shortest-path BFS in TacSimGameCoordinator
    /// so multi-room sprints and threshold crossings never miss or double-fire exit aliases.
    /// </summary>
    public sealed class ExitPortalTrigger : MonoBehaviour
    {
        public string roomA;
        public string roomB;
        public string commandAToB;
        public string commandBToA;
        public string labelAToB;
        public string labelBToA;
        public Vector3 axisAToB = Vector3.forward;
        public bool isLocked;

        public Action<ExitPortalTrigger, string, string> OnPortalCrossed;
    }

    /// <summary>
    /// Keeps floating NPC nameplates &amp; speech bubbles facing the active camera while clamping
    /// world position inside room walls so speech text never clips into bulkheads or wall boards.
    /// </summary>
    public sealed class WorldBillboard : MonoBehaviour
    {
        public TextMeshPro NameplateText;
        public TextMeshPro SpeechBubbleText;
        public float AnchorLocalY = 2.08f;

        private float _speechExpireTime;
        private bool _hasRoomBounds;
        private float _minWorldX;
        private float _maxWorldX;
        private float _minWorldZ;
        private float _maxWorldZ;
        private float _wallMarginM = 1.45f;

        public void SetRoomSafeBounds(float minWorldX, float maxWorldX, float minWorldZ, float maxWorldZ, float wallMarginM = 1.45f)
        {
            _hasRoomBounds = true;
            _minWorldX = Mathf.Min(minWorldX, maxWorldX);
            _maxWorldX = Mathf.Max(minWorldX, maxWorldX);
            _minWorldZ = Mathf.Min(minWorldZ, maxWorldZ);
            _maxWorldZ = Mathf.Max(minWorldZ, maxWorldZ);
            _wallMarginM = Mathf.Max(0.4f, wallMarginM);
        }

        public bool IsSpeechActive => SpeechBubbleText != null && SpeechBubbleText.gameObject.activeSelf;
        public string CurrentSpeechText => SpeechBubbleText != null ? SpeechBubbleText.text : string.Empty;

        public void ShowSpeech(string line, float durationSec = 8.5f)
        {
            if (SpeechBubbleText == null) return;
            SpeechBubbleText.text = line;
            SpeechBubbleText.gameObject.SetActive(!string.IsNullOrWhiteSpace(line));
            _speechExpireTime = Time.time + durationSec;
        }

        private void LateUpdate()
        {
            if (SpeechBubbleText != null && SpeechBubbleText.gameObject.activeSelf && Time.time > _speechExpireTime)
            {
                SpeechBubbleText.gameObject.SetActive(false);
            }

            Camera cam = Camera.main;

            if (_hasRoomBounds && transform.parent != null)
            {
                Vector3 anchor = transform.parent.position + new Vector3(0f, AnchorLocalY, 0f);
                if (cam != null)
                {
                    Vector3 toCam = cam.transform.position - anchor;
                    toCam.y = 0f;
                    if (toCam.sqrMagnitude > 0.01f)
                        anchor += toCam.normalized * 0.25f;
                }

                float safeMinX = _minWorldX + _wallMarginM;
                float safeMaxX = _maxWorldX - _wallMarginM;
                float safeMinZ = _minWorldZ + _wallMarginM;
                float safeMaxZ = _maxWorldZ - _wallMarginM;

                anchor.x = safeMinX < safeMaxX ? Mathf.Clamp(anchor.x, safeMinX, safeMaxX) : (_minWorldX + _maxWorldX) * 0.5f;
                anchor.z = safeMinZ < safeMaxZ ? Mathf.Clamp(anchor.z, safeMinZ, safeMaxZ) : (_minWorldZ + _maxWorldZ) * 0.5f;

                transform.position = anchor;
            }

            if (cam != null)
            {
                Vector3 fwd = transform.position - cam.transform.position;
                fwd.y = 0f;
                if (fwd.sqrMagnitude > 0.001f)
                    transform.rotation = Quaternion.LookRotation(fwd.normalized, Vector3.up);
            }
        }
    }
}
