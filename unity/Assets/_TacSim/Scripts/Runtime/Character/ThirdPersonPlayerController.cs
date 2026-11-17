using System;
using TacSim.World;
using UnityEngine;
using UnityEngine.EventSystems;

namespace TacSim.Character
{
    /// <summary>
    /// Smooth 3rd-person character locomotion controller (camera-relative WASD + Shift sprint + stairs
    /// + contextual [E] / click interaction) designed for Ceres Freeport and ship deck interiors.
    /// </summary>
    [RequireComponent(typeof(CharacterController))]
    public sealed class ThirdPersonPlayerController : MonoBehaviour
    {
        [Header("Movement")]
        public float walkSpeedMps = 4.6f;
        public float sprintSpeedMps = 7.8f;
        public float rotationSharpness = 14f;
        public float gravityMps2 = -16f;
        public float interactionRadiusM = 3.4f;

        public Transform CameraTransform { get; set; }
        public CharacterVisualSpawner VisualSpawner { get; set; }
        public bool InputSuspended { get; set; }
        public bool IsSeatedAtHelm { get; set; }

        public WorldInteractable FocusedInteractable { get; private set; }
        public event Action<WorldInteractable> OnFocusedInteractableChanged;

        private CharacterController _cc;
        private float _verticalVelocity;
        private float _currentPlanarSpeed;
        private readonly Collider[] _overlapBuffer = new Collider[24];

        private void Awake()
        {
            _cc = GetComponent<CharacterController>();
            _cc.height = 1.8f;
            _cc.radius = 0.34f;
            _cc.center = new Vector3(0f, 0.92f, 0f);
            _cc.stepOffset = 0.48f;
            _cc.slopeLimit = 52f;
        }

        public void TeleportTo(Vector3 worldPos, float yawDeg = 0f)
        {
            if (_cc == null) _cc = GetComponent<CharacterController>();
            bool wasEnabled = _cc.enabled;
            _cc.enabled = false;
            transform.SetPositionAndRotation(worldPos, Quaternion.Euler(0f, yawDeg, 0f));
            _verticalVelocity = 0f;
            _currentPlanarSpeed = 0f;
            _cc.enabled = wasEnabled;
        }

        private void Update()
        {
            float dt = Time.deltaTime;
            if (IsSeatedAtHelm)
            {
                if (VisualSpawner != null)
                    VisualSpawner.UpdateLocomotion(0f, false, true, dt);
                UpdateFocusedInteractable();
                if (!InputSuspended && Input.GetKeyDown(KeyCode.E) && FocusedInteractable != null)
                    FocusedInteractable.TriggerInteraction();
                return;
            }

            Vector2 moveInput = Vector2.zero;
            bool sprinting = false;

            if (!InputSuspended)
            {
                if (Input.GetKey(KeyCode.W) || Input.GetKey(KeyCode.UpArrow)) moveInput.y += 1f;
                if (Input.GetKey(KeyCode.S) || Input.GetKey(KeyCode.DownArrow)) moveInput.y -= 1f;
                if (Input.GetKey(KeyCode.D) || Input.GetKey(KeyCode.RightArrow)) moveInput.x += 1f;
                if (Input.GetKey(KeyCode.A) || Input.GetKey(KeyCode.LeftArrow)) moveInput.x -= 1f;
                if (moveInput.sqrMagnitude > 1f) moveInput.Normalize();
                sprinting = Input.GetKey(KeyCode.LeftShift) || Input.GetKey(KeyCode.RightShift);
            }

            Vector3 camForward = Vector3.forward;
            Vector3 camRight = Vector3.right;
            if (CameraTransform != null)
            {
                camForward = Vector3.ProjectOnPlane(CameraTransform.forward, Vector3.up).normalized;
                if (camForward.sqrMagnitude < 0.001f) camForward = Vector3.forward;
                camRight = Vector3.Cross(Vector3.up, camForward).normalized;
            }

            Vector3 desiredDir = (camRight * moveInput.x + camForward * moveInput.y);
            float targetSpeed = desiredDir.magnitude * (sprinting ? sprintSpeedMps : walkSpeedMps);
            _currentPlanarSpeed = Mathf.MoveTowards(_currentPlanarSpeed, targetSpeed, 28f * dt);

            if (desiredDir.sqrMagnitude > 0.001f)
            {
                Quaternion targetRot = Quaternion.LookRotation(desiredDir.normalized, Vector3.up);
                transform.rotation = Quaternion.Slerp(transform.rotation, targetRot, 1f - Mathf.Exp(-rotationSharpness * dt));
            }

            if (_cc.isGrounded && _verticalVelocity < 0f)
                _verticalVelocity = -2f;
            else
                _verticalVelocity += gravityMps2 * dt;

            Vector3 velocity = desiredDir.normalized * _currentPlanarSpeed + Vector3.up * _verticalVelocity;
            if (_cc.enabled)
                _cc.Move(velocity * dt);

            if (VisualSpawner != null)
                VisualSpawner.UpdateLocomotion(_currentPlanarSpeed, sprinting && _currentPlanarSpeed > walkSpeedMps + 0.5f, false, dt);

            UpdateFocusedInteractable();

            if (!InputSuspended)
            {
                if (Input.GetKeyDown(KeyCode.E) && FocusedInteractable != null)
                {
                    FocusedInteractable.TriggerInteraction();
                }
                else if (Input.GetMouseButtonDown(0) && !IsPointerOverUi() && CameraTransform != null)
                {
                    var cam = CameraTransform.GetComponent<Camera>();
                    if (cam != null)
                    {
                        Ray ray = cam.ScreenPointToRay(Input.mousePosition);
                        if (Physics.Raycast(ray, out RaycastHit hit, 14f))
                        {
                            var interactable = hit.collider.GetComponentInParent<WorldInteractable>();
                            if (interactable != null && Vector3.Distance(transform.position, interactable.transform.position) <= 8f)
                            {
                                interactable.TriggerInteraction();
                            }
                        }
                    }
                }
            }
        }

        private static bool IsPointerOverUi()
        {
            return EventSystem.current != null && EventSystem.current.IsPointerOverGameObject();
        }

        private void UpdateFocusedInteractable()
        {
            Vector3 eye = transform.position + Vector3.up * 1.1f;
            int count = Physics.OverlapSphereNonAlloc(eye, interactionRadiusM, _overlapBuffer);

            WorldInteractable best = null;
            float bestScore = float.MaxValue;
            Vector3 lookDir = CameraTransform != null ? CameraTransform.forward : transform.forward;

            for (int i = 0; i < count; i++)
            {
                var col = _overlapBuffer[i];
                if (col == null) continue;
                var candidate = col.GetComponentInParent<WorldInteractable>();
                if (candidate == null || !candidate.isActiveAndEnabled) continue;

                Vector3 delta = (candidate.transform.position + Vector3.up * 0.6f) - eye;
                float dist = delta.magnitude;
                if (dist > candidate.maxInteractDistance) continue;

                float anglePenalty = dist > 0.1f ? (1f - Vector3.Dot(lookDir, delta / dist)) * 1.6f : 0f;
                float score = dist + anglePenalty;
                if (score < bestScore)
                {
                    bestScore = score;
                    best = candidate;
                }
            }

            if (best != FocusedInteractable)
            {
                FocusedInteractable = best;
                OnFocusedInteractableChanged?.Invoke(best);
            }
        }
    }
}
