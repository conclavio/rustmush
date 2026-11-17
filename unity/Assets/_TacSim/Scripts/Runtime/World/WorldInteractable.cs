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
    /// Keeps floating NPC nameplates &amp; speech bubbles facing the active camera.
    /// </summary>
    public sealed class WorldBillboard : MonoBehaviour
    {
        public TextMeshPro NameplateText;
        public TextMeshPro SpeechBubbleText;
        private float _speechExpireTime;

        public void ShowSpeech(string line, float durationSec = 7.5f)
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
