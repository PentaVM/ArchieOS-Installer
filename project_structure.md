# Estructura del Proyecto

```text
Listado de rutas de carpetas para el volumen Datos
El número de serie del volumen es 32FD-F8A1
E:.
|   .env.example
|   .gitattributes
|   .gitignore
|   ARCHITECTURE.md
|   generate_tree.py
|   HACKATHON.md
|   LICENSE
|   NOTICE
|   project_structure.md
|   README.md
|   
+---.github
|   \---workflows
|           ci.yml
|           
+---capture-agents
|   |   README.md
|   |   
|   +---android
|   |   |   build.gradle.kts
|   |   |   
|   |   \---src
|   |       \---main
|   |           |   AndroidManifest.xml
|   |           |   
|   |           \---kotlin
|   |               \---os
|   |                   \---archie
|   |                       \---capture
|   |                               ActorSession.kt
|   |                               AgentConfig.kt
|   |                               CaptureEvent.kt
|   |                               EventPoster.kt
|   |                               NotificationCaptureService.kt
|   |                               PairingActivity.kt
|   |                               
|   \---browser
|           background.js
|           manifest.json
|           popup.html
|           popup.js
|           
+---core
|   |   Cargo.lock
|   |   Cargo.toml
|   |   README.md
|   |   
|   \---crates
|       +---archie-cerebra
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       lib.rs
|       |   |       observation.rs
|       |   |       pass.rs
|       |   |       
|       |   \---tests
|       |           loop_order.rs
|       |           
|       +---archie-cli
|       |   |   Cargo.toml
|       |   |   
|       |   \---src
|       |           main.rs
|       |           
|       +---archie-core
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       boot.rs
|       |   |       cognitive_context.rs
|       |   |       config.rs
|       |   |       control.rs
|       |   |       cortex.rs
|       |   |       dispatch.rs
|       |   |       endpoint.rs
|       |   |       execution.rs
|       |   |       hardware.rs
|       |   |       hotplug.rs
|       |   |       lib.rs
|       |   |       runtime.rs
|       |   |       task_service.rs
|       |   |       
|       |   \---tests
|       |           cognitive_context.rs
|       |           control_framing.rs
|       |           control_roundtrip.rs
|       |           cortex_projection.rs
|       |           persistent_node.rs
|       |           two_node_fabric.rs
|       |           
|       +---archie-displayd
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       bridge.rs
|       |   |       geofabric.rs
|       |   |       http.rs
|       |   |       kernel.rs
|       |   |       lib.rs
|       |   |       main.rs
|       |   |       snapshot.rs
|       |   |       world.rs
|       |   |       
|       |   \---tests
|       |           reconnect.rs
|       |           
|       +---archie-execution
|       |   |   Cargo.toml
|       |   |   
|       |   \---src
|       |           checkpoint.rs
|       |           client.rs
|       |           lib.rs
|       |           lifecycle.rs
|       |           protocol.rs
|       |           spec.rs
|       |           
|       +---archie-executord
|       |   |   Cargo.toml
|       |   |   
|       |   \---src
|       |           cgroups.rs
|       |           dispatch.rs
|       |           events.rs
|       |           journal.rs
|       |           landlock.rs
|       |           launcher.rs
|       |           lib.rs
|       |           main.rs
|       |           preempt.rs
|       |           process.rs
|       |           recovery.rs
|       |           server.rs
|       |           supervisor.rs
|       |           
|       +---archie-fabric
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       backing.rs
|       |   |       discovery.rs
|       |   |       enrolment_plan.rs
|       |   |       envelope.rs
|       |   |       identity.rs
|       |   |       lib.rs
|       |   |       membership.rs
|       |   |       message.rs
|       |   |       transport.rs
|       |   |       
|       |   \---tests
|       |           enrolment_plan.rs
|       |           isolation.rs
|       |           membership.rs
|       |           
|       +---archie-geofabric
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       catalogue.rs
|       |   |       lib.rs
|       |   |       
|       |   \---tests
|       |           fabric.rs
|       |           
|       +---archie-installer
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |   |   answers.rs
|       |   |   |   config.rs
|       |   |   |   disk.rs
|       |   |   |   lib.rs
|       |   |   |   main.rs
|       |   |   |   mapdata.rs
|       |   |   |   map_cache.rs
|       |   |   |   onboarding.rs
|       |   |   |   plan.rs
|       |   |   |   probe.rs
|       |   |   |   
|       |   |   \---bin
|       |   |           map-fetch.rs
|       |   |           
|       |   \---tests
|       |           answers.rs
|       |           disk.rs
|       |           mapdata.rs
|       |           onboarding.rs
|       |           plan.rs
|       |           probe.rs
|       |           
|       +---archie-mcp
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       gate.rs
|       |   |       lib.rs
|       |   |       manifest.rs
|       |   |       protocol.rs
|       |   |       transport.rs
|       |   |       
|       |   \---tests
|       |           gate.rs
|       |           transport.rs
|       |           
|       +---archie-memory
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       admission.rs
|       |   |       analysis.rs
|       |   |       claim.rs
|       |   |       cluster.rs
|       |   |       cognitive.rs
|       |   |       correction.rs
|       |   |       detector.rs
|       |   |       evidence.rs
|       |   |       execution.rs
|       |   |       floor.rs
|       |   |       graph.rs
|       |   |       lib.rs
|       |   |       shared_thought.rs
|       |   |       storage.rs
|       |   |       
|       |   \---tests
|       |           admission.rs
|       |           analysis.rs
|       |           cortex_wire.rs
|       |           floor.rs
|       |           protocol_agreement.rs
|       |           
|       +---archie-model-runtime
|       |   |   Cargo.toml
|       |   |   
|       |   \---src
|       |           lib.rs
|       |           main.rs
|       |           
|       +---archie-observability
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |   |   audit.rs
|       |   |   |   cognition.rs
|       |   |   |   lib.rs
|       |   |   |   metrics.rs
|       |   |   |   
|       |   |   \---kernel
|       |   |           cgroup.rs
|       |   |           mod.rs
|       |   |           psi.rs
|       |   |           thermal.rs
|       |   |           udev.rs
|       |   |           
|       |   \---tests
|       |           cognition.rs
|       |           
|       +---archie-policy
|       |   |   Cargo.toml
|       |   |   
|       |   \---src
|       |           lib.rs
|       |           
|       +---archie-postgres
|       |   |   Cargo.toml
|       |   |   README.md
|       |   |   
|       |   +---src
|       |   |       correction.rs
|       |   |       execution.rs
|       |   |       lib.rs
|       |   |       schema.sql
|       |   |       
|       |   \---tests
|       |           persistence.rs
|       |           
|       +---archie-registry
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       lib.rs
|       |   |       models.rs
|       |   |       modules.rs
|       |   |       providers.rs
|       |   |       resource.rs
|       |   |       
|       |   \---tests
|       |           provider_tests.rs
|       |           registry_tests.rs
|       |           
|       +---archie-router
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       lib.rs
|       |   |       
|       |   \---tests
|       |           roles.rs
|       |           
|       +---archie-scheduler
|       |   |   Cargo.toml
|       |   |   
|       |   \---src
|       |           lib.rs
|       |           queue.rs
|       |           
|       +---archie-speech
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       adapter.rs
|       |   |       cognitive.rs
|       |   |       floor.rs
|       |   |       lib.rs
|       |   |       session.rs
|       |   |       
|       |   \---tests
|       |           adapter.rs
|       |           cognitive.rs
|       |           floor.rs
|       |           session.rs
|       |           
|       +---archie-state
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |   |   adapters.rs
|       |   |   |   cognitive.rs
|       |   |   |   lib.rs
|       |   |   |   paths.rs
|       |   |   |   query.rs
|       |   |   |   schema.sql
|       |   |   |   snapshots.rs
|       |   |   |   store.rs
|       |   |   |   timeseries.rs
|       |   |   |   
|       |   |   \---bin
|       |   |           archie-db.rs
|       |   |           
|       |   \---tests
|       |           cognitive.rs
|       |           durability.rs
|       |           execution.rs
|       |           graph.rs
|       |           shared_thought.rs
|       |           snapshots.rs
|       |           store_tests.rs
|       |           timeseries.rs
|       |           
|       +---archie-task-runner
|       |   |   Cargo.toml
|       |   |   
|       |   \---src
|       |           abi.rs
|       |           handoff.rs
|       |           lib.rs
|       |           main.rs
|       |           plan.rs
|       |           rights.rs
|       |           ruleset.rs
|       |           
|       +---archie-types
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       capability.rs
|       |   |       error.rs
|       |   |       ids.rs
|       |   |       lib.rs
|       |   |       model.rs
|       |   |       model_role.rs
|       |   |       module.rs
|       |   |       node.rs
|       |   |       task.rs
|       |   |       time.rs
|       |   |       
|       |   \---tests
|       |           model_role.rs
|       |           protocols.rs
|       |           
|       +---archie-vault
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       gateway.rs
|       |   |       lib.rs
|       |   |       reference.rs
|       |   |       store.rs
|       |   |       
|       |   \---tests
|       |           privacy.rs
|       |           
|       +---archie-worldd
|       |   |   Cargo.toml
|       |   |   
|       |   +---src
|       |   |       http.rs
|       |   |       layers.rs
|       |   |       lib.rs
|       |   |       main.rs
|       |   |       
|       |   \---tests
|       |       |   layers.rs
|       |       |   
|       |       \---fixtures
|       |               usgs_all_hour.geojson
|       |               
|       \---archied
|           |   Cargo.toml
|           |   
|           +---src
|           |       main.rs
|           |       notify.rs
|           |       
|           \---tests
|                   distro_contract.rs
|                   packaging.rs
|                   
+---cortex
|   |   CMakeLists.txt
|   |   
|   +---include
|   |   \---archie
|   |       \---cortex
|   |               contracts.hpp
|   |               intelligence.hpp
|   |               ipc.hpp
|   |               
|   +---src
|   |       contracts.cpp
|   |       intelligence.cpp
|   |       ipc.cpp
|   |       main.cpp
|   |       
|   \---tests
|           benchmark_retrieval.cpp
|           test_contracts.cpp
|           test_daemon.cpp
|           test_intelligence.cpp
|           
+---display
|   +---installer
|   |   |   CMakeLists.txt
|   |   |   main.cpp
|   |   |   README.md
|   |   |   Wizard.cpp
|   |   |   Wizard.h
|   |   |   
|   |   \---qml
|   |           Main.qml
|   |           
|   +---native
|   |   |   CMakeLists.txt
|   |   |   
|   |   +---assets
|   |   |       earth-blue-marble.png
|   |   |       earth-bmng-5400.jpg
|   |   |       README.md
|   |   |       
|   |   +---qml
|   |   |       Main.qml
|   |   |       
|   |   +---shaders
|   |   |       atmosphere.frag
|   |   |       atmosphere.vert
|   |   |       star.frag
|   |   |       star.vert
|   |   |       
|   |   \---src
|   |           ArchieBridge.cpp
|   |           ArchieBridge.h
|   |           main.cpp
|   |           MeshGeometry.cpp
|   |           MeshGeometry.h
|   |           Payload.cpp
|   |           Payload.h
|   |           
|   +---osm
|   |   |   CMakeLists.txt
|   |   |   
|   |   +---include
|   |   |   \---archie
|   |   |       \---osm
|   |   |               building.hpp
|   |   |               
|   |   +---src
|   |   |       blob.cpp
|   |   |       blob.hpp
|   |   |       building.cpp
|   |   |       protobuf_reader.hpp
|   |   |       
|   |   \---tests
|   |           harness.hpp
|   |           test_building.cpp
|   |           test_protobuf.cpp
|   |           
|   \---visual
|       |   CMakeLists.txt
|       |   README.md
|       |   
|       +---include
|       |   \---archie
|       |       \---visual
|       |               budget.hpp
|       |               camera.hpp
|       |               events.hpp
|       |               freshness.hpp
|       |               geo.hpp
|       |               graph.hpp
|       |               history.hpp
|       |               ingest.hpp
|       |               layout.hpp
|       |               lod.hpp
|       |               mesh.hpp
|       |               observatory.hpp
|       |               palette.hpp
|       |               scene.hpp
|       |               value.hpp
|       |               
|       +---src
|       |       budget.cpp
|       |       camera.cpp
|       |       events.cpp
|       |       geo.cpp
|       |       graph.cpp
|       |       history.cpp
|       |       ingest.cpp
|       |       layout.cpp
|       |       lod.cpp
|       |       mesh.cpp
|       |       observatory.cpp
|       |       palette.cpp
|       |       scene.cpp
|       |       value.cpp
|       |       
|       +---tests
|       |   |   harness.hpp
|       |   |   test_budget.cpp
|       |   |   test_camera.cpp
|       |   |   test_events.cpp
|       |   |   test_freshness.cpp
|       |   |   test_geo.cpp
|       |   |   test_graph.cpp
|       |   |   test_history.cpp
|       |   |   test_ingest.cpp
|       |   |   test_layout.cpp
|       |   |   test_lod.cpp
|       |   |   test_mesh.cpp
|       |   |   test_observatory.cpp
|       |   |   test_palette.cpp
|       |   |   test_scene.cpp
|       |   |   test_world.cpp
|       |   |   
|       |   \---golden
|       |           scenario.txt
|       |           
|       \---tools
|               bench.cpp
|               probe.cpp
|               render_svg.cpp
|               sync-check-count.sh
|               
+---distro
|   |   README.md
|   |   
|   +---archiso
|   |   |   build.sh
|   |   |   packages.x86_64
|   |   |   pacman.conf
|   |   |   profiledef.sh
|   |   |   README.md
|   |   |   
|   |   +---airootfs
|   |   |   |   README.md
|   |   |   |   
|   |   |   +---etc
|   |   |   |   |   hostname
|   |   |   |   |   shadow
|   |   |   |   |   
|   |   |   |   +---archie
|   |   |   |   |       archie.toml
|   |   |   |   |       
|   |   |   |   +---mkinitcpio.conf.d
|   |   |   |   |       archiso.conf
|   |   |   |   |       
|   |   |   |   +---mkinitcpio.d
|   |   |   |   |       linux.preset
|   |   |   |   |       
|   |   |   |   \---systemd
|   |   |   |       +---network
|   |   |   |       |       20-wired.network
|   |   |   |       |       
|   |   |   |       \---system
|   |   |   |           |   archie-executord.service
|   |   |   |           |   archied.service
|   |   |   |           |   
|   |   |   |           \---getty@tty1.service.d
|   |   |   |                   autologin.conf
|   |   |   |                   
|   |   |   \---usr
|   |   |       +---lib
|   |   |       |   +---systemd
|   |   |       |   |   \---system-preset
|   |   |       |   |           50-archie-os.preset
|   |   |       |   |           
|   |   |       |   +---sysusers.d
|   |   |       |   |       archie.conf
|   |   |       |   |       
|   |   |       |   \---tmpfiles.d
|   |   |       |           archie.conf
|   |   |       |           
|   |   |       \---share
|   |   |           \---doc
|   |   |               \---archie-os
|   |   |                       README.md
|   |   |                       
|   |   +---efiboot
|   |   |   \---loader
|   |   |       |   loader.conf
|   |   |       |   
|   |   |       \---entries
|   |   |               01-archieos-linux.conf
|   |   |               02-archieos-install.conf
|   |   |               
|   |   \---syslinux
|   |           syslinux-linux.cfg
|   |           syslinux.cfg
|   |           
|   +---boot
|   |       README.md
|   |       
|   +---cortex
|   |       archie-cortex.service
|   |       archie-cortex.socket
|   |       archie-cortex.sysusers
|   |       archie-cortex.tmpfiles
|   |       
|   +---database
|   |       arch-packages.lock
|   |       archie-database.service
|   |       archie-postgres-init.service
|   |       archie-postgres.service
|   |       compatibility.json
|   |       initialize-cluster.sh
|   |       pacman.conf
|   |       preflight.sql
|   |       provision-node.sh
|   |       README.md
|   |       stage-reviewed-packages.sh
|   |       stage-test-extensions.ps1
|   |       test-postgres.sh
|   |       wait-postgres.sh
|   |       
|   +---display
|   |       archie-display.service
|   |       archie-display.sysusers
|   |       archie-displayd.service
|   |       archie-worldd.service
|   |       README.md
|   |       
|   +---filesystem
|   |       README.md
|   |       
|   +---installer
|   |       archie-installer.service
|   |       archie-wizard.desktop
|   |       finalize-target.sh
|   |       packages.x86_64
|   |       README.md
|   |       wizard-session.sh
|   |       
|   +---kernels
|   |       README.md
|   |       
|   +---nvidia
|   |       packages.x86_64
|   |       README.md
|   |       
|   \---package-manifests
|           README.md
|           
+---docs
|       ARCHITECTURE.md
|       CEREBRA.md
|       RFC-001-CONTROL-ENDPOINT-LIFECYCLE.md
|       RFC-002-PERSISTENT-LOCAL-NODE.md
|       RFC-003-TWO-NODE-FABRIC.md
|       RFC-004-ARCH-NATIVE-SERVICE.md
|       RFC-005-ARCHISO-RELEASE-PROFILE.md
|       RFC-006-DISPLAY-ISOLATION.md
|       RFC-007-KERNEL-INTEGRATION.md
|       RFC-008-SENTRY-WORLD-MODEL.md
|       RFC-009-ANALYTICAL-CORTEX.md
|       RFC-010-OBSERVATORY-VISUAL-MODEL.md
|       RFC-011-LEARNING-ADMISSION.md
|       RFC-012-ANALYTICAL-SYNTHESIS.md
|       SECURITY.md
|       
+---modules
|   |   README.md
|   |   
|   +---cognition
|   |       critic.toml
|   |       hypothesis-generator.toml
|   |       planner.toml
|   |       researcher.toml
|   |       verifier.toml
|   |       
|   +---core
|   |       archie.toml
|   |       forge.toml
|   |       memory.toml
|   |       nova.toml
|   |       router.toml
|   |       sentry.toml
|   |       
|   +---cyber
|   |       exposure.toml
|   |       internet-health.toml
|   |       vulnerability.toml
|   |       
|   +---devices
|   |       suit.toml
|   |       
|   +---engineering
|   |       cad.toml
|   |       cfd.toml
|   |       surrogate.toml
|   |       
|   +---fusion
|   |       corroborate.toml
|   |       detect.toml
|   |       normalize.toml
|   |       
|   +---markets
|   |       instruments.toml
|   |       macro.toml
|   |       
|   +---media
|   |       news.toml
|   |       provenance.toml
|   |       
|   +---science
|   |       physics.toml
|   |       
|   \---sentry
|           airspace.toml
|           aviation.toml
|           conflict.toml
|           geophysical.toml
|           infrastructure.toml
|           maritime.toml
|           orbital.toml
|           
+---protocols
|   |   analytical-signal.schema.json
|   |   capture-event.schema.json
|   |   claim.schema.json
|   |   cortex-cognitive-event.schema.json
|   |   cortex-context-wire.md
|   |   cortex-correction-trajectory.schema.json
|   |   detector.schema.json
|   |   event.schema.json
|   |   learning-candidate.schema.json
|   |   observation.schema.json
|   |   person-public-profile.schema.json
|   |   provenance.schema.json
|   |   provider.schema.json
|   |   README.md
|   |   simulation-run.schema.json
|   |   
|   \---fixtures
|           cortex-focus-request-v2.hex
|           cortex-focus.json
|           cortex-graph-v1.hex
|           cortex-graph.json
|           cortex-prospective-query.json
|           cortex-prospective-request-v3.hex
|           cortex-query.json
|           
+---providers
|   |   README.md
|   |   
|   +---data
|   |   |   README.md
|   |   |   
|   |   +---cyber
|   |   |       cisa-kev.toml
|   |   |       first-epss.toml
|   |   |       ripe-ris.toml
|   |   |       
|   |   +---markets
|   |   |       fred.toml
|   |   |       sec-edgar.toml
|   |   |       
|   |   \---world
|   |           adsb-lol.toml
|   |           celestrak.toml
|   |           gdacs.toml
|   |           nasa-firms.toml
|   |           nasa-gibs.toml
|   |           noaa-swpc.toml
|   |           nws-alerts.toml
|   |           opensky.toml
|   |           usgs-earthquakes.toml
|   |           
|   \---nvidia
|           README.md
|           
+---sentry
|   |   CMakeLists.txt
|   |   
|   +---include
|   |   \---sentry
|   |       +---camera
|   |       |       authority.hpp
|   |       |       connection.hpp
|   |       |       
|   |       +---capture
|   |       |       activity.hpp
|   |       |       ingest.hpp
|   |       |       
|   |       +---identity
|   |       |       override.hpp
|   |       |       profile.hpp
|   |       |       relationship.hpp
|   |       |       retention.hpp
|   |       |       subject.hpp
|   |       |       wanda.hpp
|   |       |       
|   |       +---ingest
|   |       |       chat_export.hpp
|   |       |       
|   |       +---policy
|   |       |       consent.hpp
|   |       |       override.hpp
|   |       |       
|   |       +---providers
|   |       |       manifest.hpp
|   |       |       
|   |       +---runtime
|   |       |       camera_worker.hpp
|   |       |       detector.hpp
|   |       |       factory.hpp
|   |       |       frame_source.hpp
|   |       |       
|   |       +---store
|   |       |       graph_store.hpp
|   |       |       
|   |       +---vision
|   |       |       track.hpp
|   |       |       
|   |       \---world
|   |               epistemic.hpp
|   |               geo.hpp
|   |               provenance.hpp
|   |               time.hpp
|   |               
|   +---src
|   |   +---camera
|   |   |       authority.cpp
|   |   |       connection.cpp
|   |   |       
|   |   +---capture
|   |   |       activity.cpp
|   |   |       ingest.cpp
|   |   |       
|   |   +---cli
|   |   |       main.cpp
|   |   |       
|   |   +---identity
|   |   |       override.cpp
|   |   |       profile.cpp
|   |   |       relationship.cpp
|   |   |       retention.cpp
|   |   |       subject.cpp
|   |   |       wanda.cpp
|   |   |       
|   |   +---ingest
|   |   |       chat_export.cpp
|   |   |       
|   |   +---policy
|   |   |       consent.cpp
|   |   |       override.cpp
|   |   |       
|   |   +---providers
|   |   |       manifest.cpp
|   |   |       
|   |   +---runtime
|   |   |       camd_main.cpp
|   |   |       camera_worker.cpp
|   |   |       gstreamer_source.cpp
|   |   |       onnx_detector.cpp
|   |   |       
|   |   +---store
|   |   |       graph_store.cpp
|   |   |       
|   |   +---vision
|   |   |       track.cpp
|   |   |       
|   |   \---world
|   |           epistemic.cpp
|   |           geo.cpp
|   |           provenance.cpp
|   |           time.cpp
|   |           
|   +---tests
|   |       harness.hpp
|   |       test_camera_worker.cpp
|   |       test_capture.cpp
|   |       test_capture_ingest.cpp
|   |       test_connection.cpp
|   |       test_geo.cpp
|   |       test_identity.cpp
|   |       test_ingest.cpp
|   |       test_invariants.cpp
|   |       test_override.cpp
|   |       test_profile.cpp
|   |       test_provenance.cpp
|   |       test_providers.cpp
|   |       test_retention.cpp
|   |       test_store.cpp
|   |       test_vision.cpp
|   |       test_wanda.cpp
|   |       
|   \---tools
|           wanda_snapshot.cpp
|           
\---tools
    |   README.md
    |   
    +---engineering-dashboard
    |   |   .env.example
    |   |   .gitignore
    |   |   next-env.d.ts
    |   |   next.config.ts
    |   |   package-lock.json
    |   |   package.json
    |   |   README.md
    |   |   tsconfig.json
    |   |   
    |   +---app
    |   |   |   globals.css
    |   |   |   layout.tsx
    |   |   |   
    |   |   +---api
    |   |   |   \---[action]
    |   |   |           route.ts
    |   |   |           
    |   |   +---login
    |   |   |       page.tsx
    |   |   |       
    |   |   \---[[...section]]
    |   |           page.tsx
    |   |           
    |   +---lib
    |   |       auth.ts
    |   |       events.ts
    |   |       log-redactor.ts
    |   |       outbox.ts
    |   |       store.ts
    |   |       
    |   +---scripts
    |   |       local-node.mjs
    |   |       publish-status.ts
    |   |       run-check.ts
    |   |       
    |   \---tests
    |           browser.mjs
    |           data-preflight.test.ts
    |           extensions.test.ts
    |           log-redactor.test.ts
    |           outbox.test.ts
    |           postgres.test.ts
    |           security.test.ts
    |           with-postgres.mjs
    |           
    \---owner-companion
        |   native-auth.mjs
        |   package-lock.json
        |   package.json
        |   README.md
        |   server.mjs
        |   service.mjs
        |   setup.mjs
        |   
        +---public
        |       app.js
        |       index.html
        |       
        \---tests
                browser.test.mjs
                security.test.mjs
                
```
