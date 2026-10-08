package com.clashedge.android

import android.app.Application
import com.clashedge.android.core.ProxyCoordinator
import com.clashedge.android.config.AppConfigStore
import com.clashedge.android.repository.ProfileRepository
import com.clashedge.android.subscription.SubscriptionRepository
import com.clashedge.android.util.Logger

/**
 * App-scoped dependency container.
 *
 * The UI never talks to the VpnService / Mihomo core directly; it goes through
 * [ProxyCoordinator] whose StateFlow reflects the real core state, so the UI can
 * never show "connected" while the actual proxy is down.
 */
class ClashEdgeApp : Application() {

    lateinit var appConfig: AppConfigStore
        private set
    lateinit var profiles: ProfileRepository
        private set
    lateinit var subscriptions: SubscriptionRepository
        private set
    lateinit var coordinator: ProxyCoordinator
        private set

    override fun onCreate() {
        super.onCreate()
        Logger.init(this)
        appConfig = AppConfigStore(this)
        subscriptions = SubscriptionRepository(appConfig)
        profiles = ProfileRepository(this, subscriptions, appConfig)
        coordinator = ProxyCoordinator(this, appConfig, profiles)
    }
}
