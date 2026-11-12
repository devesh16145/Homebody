// Homebody for JetBrains editors (IntelliJ IDEA, PyCharm, WebStorm and the rest).
// Plain Java, no extra libraries: what it needs (JSON, the editor's own interface) is part of
// the editor it is installed into.
plugins {
    id("java")
    id("org.jetbrains.intellij.platform") version "2.2.1"
}

group = "app.homebody"
version = "0.1.0"

repositories {
    mavenCentral()
    intellijPlatform {
        defaultRepositories()
    }
}

dependencies {
    intellijPlatform {
        intellijIdeaCommunity("2024.2.4")
        pluginVerifier()
    }
}

java {
    sourceCompatibility = JavaVersion.VERSION_21
    targetCompatibility = JavaVersion.VERSION_21
}

intellijPlatform {
    // JetBrains' own checker reads the built plugin against another editor's code and lists
    // anything the plugin uses that is not there:  gradle verifyPlugin -PverifyAgainst=/Applications/DataGrip.app
    pluginVerification {
        ides {
            providers.gradleProperty("verifyAgainst").orNull?.let { local(it) }
        }
    }
    buildSearchableOptions = false
    pluginConfiguration {
        ideaVersion {
            sinceBuild = "242"
            untilBuild = provider { null }
        }
    }
}

// The plugin, exercised inside a real running editor against the Homebody on this computer:
//   test/in-editor/run.sh   (it passes -PselfCheck=<result file> -PselfCheckProject=<folder>)
// Only then is src/selfcheck compiled in; a normal build does not contain it.
val selfCheck = providers.gradleProperty("selfCheck")
if (selfCheck.isPresent) {
    sourceSets["main"].java.srcDir("src/selfcheck/java")
    tasks.named<org.jetbrains.intellij.platform.gradle.tasks.RunIdeTask>("runIde") {
        jvmArgs(
            "-Dhomebody.selfcheck=${selfCheck.get()}",
            // A fresh editor asks about privacy, trust and tips before it opens anything.
            "-Djb.privacy.policy.text=<!--999.999-->",
            "-Djb.consents.confirmation.enabled=false",
            "-Didea.trust.all.projects=true",
            "-Dide.show.tips.on.startup.default.value=false",
            "-Dhomebody.selfcheck.place=${providers.gradleProperty("selfCheckPlace").getOrElse("example.net")}",
        )
        args(providers.gradleProperty("selfCheckProject").get())
    }
}
